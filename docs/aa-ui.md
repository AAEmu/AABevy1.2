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
| `lua/bounds.rs` | synchronous native event33, source then live anchor dependents |
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
| `prof.rs` | opt-in frame profiler (`AA_UI_PROFILE=1`: ms per part of the UI frame, every object that changed per frame (`dirty objects`), the `touch` / `repaint` sites, batch uploads, glyph atlas uploads, frames over 35 ms, the costly per-frame handlers; `=2`: per native method, and which ones wrote; `=3`: also logs what differs in each changed object) |
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
access bumps; the grid pass skips frames without a net change. Bound notifications run in
native size setters, independently of the rendering change comparison. Text
layouts are cached by font, size, text and width (`Fonts::layout_size` measures without a copy).
Cost of the in-game HUD (release, standing at the Nuian spawn, `AA_PROFILE=1`, main thread
`update` slice of aa-client, 3 alternating runs each): 4.4-6.6 ms before this (the scripts'
every-frame rewrites rebuilt the meshes every frame), 1.6-2.2 ms after, against 0.7-1.4 ms with
`--no-hud`, so the HUD went from about 4.3 ms to about 1 ms; the aa-ui frame itself 0.6-0.8 ms,
about 0.45 ms of it the unit frame scripts' own `OnUpdate`. `AA_UI_PROFILE=1` prints
the split every 3 s. Glyphs live in one 2048x2048 atlas (which also holds the
white texel for solid quads). Textures are the pak's DDS files, decoded by Bevy (DXT1, DXT5,
A8R8G8B8 so far).

A rebuild re-collects all quads but uploads only the batches whose quads changed:
`sync_batches` keeps the batch entities and meshes of the last upload (`UiRenderState::batches`)
and matches the new batch list by position. An unchanged batch (same texture, same quad hash)
is left alone; a changed one gets its mesh replaced under the same handle, and its material
swapped if its texture changed; only a longer list spawns entities. Right after entering the
world the HUD really does change every frame for about 4 s (the zone-name banner
`subZoneMsgWindow` fades in and out, and an expiring buff icon blinks through `SetAlpha`), so it
rebuilds 50 to 60 times a second. Before, every rebuild despawned and respawned all ~100 batch
entities with new meshes (about 5,000 new meshes a second); now 4 or 5 batches per frame are
uploaded and no mesh is created (dev build, lane server, `AA_UI_PROFILE=1`, 3 interleaved runs
each: `+mesh` in the first 6 s in game 22,500 to 24,300 before, 1 to 4 after; `sync_meshes`
0.33 -> 0.23 ms per rebuild frame). The 45 to 60 ms frames about once a second in that window
come from `--enter-shots` (each screenshot is one), not from the HUD: without it neither build
has a frame over 35 ms in the first 15 s.

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
visible widgets. `OnBoundChanged(self)` is event33: an admitted size setter first finishes
event32 anchor propagation, then sends event33 to the source and recursively to its live
anchor dependents. Identical sizes still notify; registration/first layout, anchor moves,
removal and native scale alone do not. Hidden widgets participate. See the native bound
dispatch evidence below for silent controls, drawable barriers and reentry/lifetime limits.
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
* UI texts: `UiConfig { ui_texts_db: Some("<server>/Data/compact.sqlite3".into()), .. }`
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
  | `AAUI.SlotContent(slotType, slotIdx)` | `slot:EstablishSlot` (action bars: `ISLOT_ACTION`, 1..) | nil, or `{ type = "skill"\|"item"\|..., id, icon = "ui/icon/x.dds", stack, grade, canUse, tooltip, extraInfo, cooldown = { remain = ms, total = ms } }` (`cooldown` optional: sweep and native f32 seconds/minutes/hours caption, counted down locally; slot totals below 200 ms skip drawing; actual option82 controls captions through `lua::slot::set_cooldown_numbers`; `refresh_slots` restarts from `remain`, absence clears it; explicit native global/own selection remains open) |
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
* `input::set_release_callback(&lua, callback)` registers one replaceable Rust callback per
  Lua state, with signature `Fn(&mlua::Lua, bevy::prelude::MouseButton) -> mlua::Result<()> +
  'static`. It runs synchronously for Left/Right after that release's GUI mouse-up, click,
  slot-use and drag continuations, before the next release and `lua::tick`. Right also finishes
  held-cursor clearing first. A delivered release still notifies over empty space or with no
  cursor; this is completion notification, with no input-consumption or operation-success result.
  The toolkit drops appdata/tree guards before flushing pending handlers, invoking the callback
  and flushing again. Errors log once and delivery continues. Replacement affects the next
  notification; nested delivery with a distinct `MouseState` stays synchronous. Registration
  returns an error with the old callback intact when app data is borrowed. Use the supplied
  Lua rather than capture the Lua state, and release application state borrows before calling
  Lua. A no-op replaces application behavior; dropping the Lua state drops its registration.
  Without registration there are no extra flushes or capture revalidation. Registered callbacks
  may retire widgets: captures and cached hover are filtered before later right/wheel delivery,
  without repicking or extra enter/leave events. Physical and virtual edges are ORed once and
  both consumed, including when both sources report the same right edge.
  `cargo test -p aa-ui --lib input::tests` covers these boundaries without a renderer or native
  window, including actual `ui_frame` edge merging and the following frame's lack of replay.
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
| generic cooldown buttons draw a sweep without a caption; slots use native f32 localized `s`/`m`/`h` captions and option82, without classifying GCD by duration | `lua/slot.rs` |
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
  player model between the equipment slot columns). Cooldown sweep tessellation remains
  approximate. Slot captions now follow executed native float/localization arithmetic;
  generic cooldown buttons have no native caption. The current content hook still lacks
  the native slot's separate global/own selection and +311/+312 state. It raises skill
  cooldown events for supplied timers; it must not infer global state or events from
  duration or the combined winning timer.
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
`<client12-unpacked>/x2game_unpacked.dll`, rather than inferred from parameter names:

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
`button:OnInput`. Wheel input goes to capture even outside its rectangle. The middle and the two side mouse buttons are keys
to the game: with a visible capture, `UiRuntime::button_event(name, pressed)` (the `ui_keyboard` system feeds it from `MouseButtonInput`,
`lua::keyboard::button_name`) dispatches `OnKeyDown` / `OnKeyUp` `mouse3` / `mouse4` / `mouse5` to the capture owner, so the original
page binds them (`SHIFT-CTRL-ALT-MOUSE4`); without a capture nothing takes them, and left and right stay clicks.

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

This read-only audit uses `<client12-unpacked>/x2game_unpacked.dll` and original `.alb` scripts.
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
RE copy `<client12-unpacked>/bin/archeage.exe`, whose bytes match the installed original.
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

The later raw/interface-base audit corrects the original hydration inference:
factory `3936b580` registers the raw actor, while `3936b11a` passes raw actor
`+8` to hydrator `398a1f90`. Its `398a218d..93` write of UnitState `+2020`
to destination `+3af8` therefore lands at raw actor `+3b00`, not the SC203
array. The old claim that the first PISH integer hydrates these GM flags was
wrong. That PISH pack remains a separate received field; it does not establish
the initial cooldown suppression byte. Actual matching SC203 mode 1 writes raw
`+3af9`, which is owner-interface `+3af1`; without that receipt the app must
keep this guard unknown. See the executed actor-base oracle
data/extract/aa-net/native-unit-actor-base-oracle.json. No UI widget API depends
on this unproved initial actor state, and no extra wire bytes are introduced.

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
false retains both. Number adapter `39259c90` requires a number; bool adapter
`39259d50` accepts the bool produced by CryScript `3550bf90`, which also converts
numeric arguments using `value != 0` (including NaN as true). Missing values,
numeric strings and other types do nothing. The initial bool-only interpretation
was corrected after checking this script-interface conversion boundary.
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
hash: constructor stores/anchor, typed argument adapters, raw setters and f32
edges, actual ButtonWidget drag producer/callback order, real tab/message lookup,
raw-zero protection, secondary split boundary and frame blinking/expiry.
GetParamAny applies the source-closed CryScript numeric-to-bool conversion;
Lua/audio/profile delivery, observer forwarding, generic move capture, secondary
split and manager preparation are explicit endpoints; remaining instructions run
unchanged. The oracle neither performs nor emulates splitting. Instructions are
in `native-chat-lock-source-20261018.txt`.

`cargo check -p aa-ui`, the example build and **37 selected tests** pass across
locking, alpha, context, method persistence, ordinary dragging and resizing.
Seven new tests exercise typed setter conversion/defaults, signed zero/NaN/infinity,
simple clicks/context while locked, callback-before-policy ordering, physical
movement and capture release, disabling during capture, retained timer state,
expiry/zero interval, removal during the drag callback and the unchanged original
chat script's artwork/timers. The aa-dev example `native_chat_lock` makes a real
pointer drag attempt and retains the original script's notification geometry.

The inspected `shots/aa-ui/native-chat-lock-20261018.png` shows the original lock
above the chat tabs at 1280x720/UI scale 0.85. It reports zero Lua errors, no
runtime warnings/errors and **224.1-283.8 FPS**; its copied process self-closes
after the screenshot. No corresponding locked-chat reference capture is present.

The bool-conversion correction passes all **15 focused locking/context/method
tests**, including numeric zero, negative zero, negative nonzero and NaN, while
strings/missing values retain the previous policy. Literal bool behavior and the
inspected capture are unchanged.

### Original dishonor numeric text token (2026-10-18)

The original character panel's `CreateDishonorPointWindow` uses
`string.format("|d%d", crimeRecord)` without a trailing semicolon. The toolkit
now decodes **`|d`/`|D`** as the native signed integer plus dishonor skull, instead
of displaying that raw control sequence. Full widget text getters remain
unchanged, literal edit text stays literal, and existing color/link markup keeps
its behavior. A `CRIME_REPORTED` event refreshes the unchanged original control.

Native dispatch `39273e58..64 -> 39274205` calls numeric parser `3926fbd0`, then
`39273190` with formatted icon kind 4. The parser consumes through semicolon or
end of string, including embedded bars/newlines, and accepts either terminator.
An empty body emits nothing; a nonempty invalid body becomes zero. `XlAtoI64`
`3301bcd0 -> 3301ad10` accepts ASCII leading whitespace/sign, stops at the first
non-digit and wraps 64-bit arithmetic. There is no saturation or floating-point
round trip. Native code first narrows each decoded code point to its low byte.
The signed MOVSX lookup at `3301ada0..a9` sees zero in all 128 preceding table
bytes, so high bytes count as digit zero; for example `|d1é2;` formats 102.
This behavior is preserved without Rust Unicode-number classification.

The initialized native table `39ac6c68` holds Dishonor kind 4 at atlas rectangle
**(62, 30, 16, 20)** in `ui/common/money_window.dds`. This was read from the
existing original process with the read-only `runtime_native.py` helper, which
verifies its module using the `StandMode` string and opens only PROCESS_VM_READ.
The small raw/decoded captures are
`native-formatted-text-icon-runtime-20261018.txt/.json` under
`data/extract/aa-ui`. This is initialized runtime configuration evidence; the
protected PE data bytes are not interpreted as default rectangles. Original
China-provider Lua changes this rectangle through
`SetTextureCoordsForFormattedText`, whose dynamic palette support remains open.

`formatted_text_oracle.py` and `native-formatted-text-oracle-20261018.json`
execute **43 native groups plus 255 leading-byte cases**, with the PE hash:
the actual text parser loop, numeric parser, XlGetChar/XlAtoI64 and comma
formatter. Allocation/memory primitives and final glyph/value layout are explicit
endpoints; numeric arithmetic, token branches and consumption execute unchanged.
Original `north_america.lua` calls `SetUseInsertComma(false)`; this slice uses
that ungrouped numeric display. The generic comma-option binding remains open.
Instructions are in `native-formatted-text-source-20261018.txt`.

This bounded fix adds the dishonor token and source rectangle to the existing
inline-icon renderer. Honor/vocation parsing, other token families, shared icon
advance/scaling/centering and locale-dependent rectangle changes are separate
parity boundaries; this milestone does not claim to have recalibrated them.

All **42 selected tests** pass with one existing ignored font-metrics test,
covering the new decoder/control, library text tests, direct drawable text,
slot state and character/item/quest input links. The four new tests check native
numeric goldens, overflow and low-byte behavior, full source retention,
color/link/literal-edit behavior, and original `CRIME_REPORTED` refresh.

The aa-dev `native_dishonor_text` example opens the unchanged full character
panel with explicit fixture crime points 80 and crime record 1234. The inspected
`shots/aa-ui/native-dishonor-character-20261018.png` shows **1234 plus the red
skull** in its original Infamy Pts row at 1280x720/UI scale 0.85. The base panel
was compared with `reference/ingame/character_info/real.png`, which has zero
crime data and therefore lacks those extra rows. The example reports zero Lua
errors, no runtime warnings/errors and **493.9-499.5 FPS**, and self-closes after
the screenshot. Its build passes.

### Native all-chat-window reset and retirement (2026-10-18)

`aa_ui::lua::chat::reset_windows(&Lua) -> usize` immediately retires all live
chat windows, including hidden and unconfigured windows, then clears shared tab
history. It returns the retired window count. The application owns the four
local configuration keys, default model and subsequent original
`OnResetAllChatWindow()` callback. No additional native Lua method or callback
is invented. Ordinary tab removal continues to retain history.

The actual InitChatWindow registration at `3945b33d` binds `394581f0`, which calls
`395083e0 -> 39507410` and returns **zero Lua results**. This is distinct from
`39508050` (ClearChatTab). `39507410` immediately destroys each registered live
window through `392aa9a0`, attempts the four local-key deletions, calls
`395072c0` to clear/reload manager configuration and history, then invokes
`OnResetAllChatWindow`. Failed key construction/deletion does not abort reset.
The callback has **zero arguments**: the stack bool pointer passed into
`39286650 -> 390243b0` is result storage for the script VM's EndCallAny, rather
than a Lua argument. Original chat_window.alb recreates its configured windows,
then restores the first window's bottom-left anchor and 430x350 extent.

`392aa9a0` unregisters the widget, removes it from its parent, clears GUI input
references through `392aa7e0`, then calls its deleting destructor. The toolkit
clears owned movement/resize capture and derived retirement/clip indices, and
prunes dead MouseState IDs before the next input dispatch. Unrelated focus,
movement capture and held items remain intact. Window destruction does not
invent an OnHide event for the destroyed window or a separate attached widget.

A preliminary source note confused the input-cleanup call at `3927e3c1` with
attachment hiding. That call is `392aa7e0`, not `3927f9b0`. Attachment hiding is
reached by native OnHide `3927ff20 -> 3927f9b0`; it is not a direct destruction
step in the traced chat/widget destructors. The direct programmatic reset of an
open attached popup remains an unresolved observer/geometry lifetime boundary.
The actual original context popup dismisses itself on the outside press
that opens another control, restoring its alpha state before reset. The example
exercises that physical dismissal. Direct programmatic reset while leaving an
unrelated top-level attached popup open does not gain an invented hide callback.

Native manager reload `395072c0` clears its history tree with `39506800` before
loading defaults/options/locks/selection. The history destructor destroys each
message vector and clears its pointers. Missing persisted extra info defaults
to selected 0/method 0; option fallback is 0.7/0.5 and locks default to zero.
Those configuration and persistence responsibilities stay with the app.
The tab, option, lock and extra-info loads are independent: missing tab data
constructs default tabs but still attempts saved extra info at `3950739c`.
`39507060 -> 39506c40` default construction populates the in-memory maps without
saving tab configuration. The extra key must not be ignored merely because the
tab key is absent.

`data/extract/aa-ui/chat_reset_oracle.py` and
`native-chat-reset-oracle-20261018.json` execute **14 native groups**, with the PE
hash: actual Init binding/order, empty/two-window trees, failed key construction,
disabled/missing callback, configuration load/default branches (including missing
tabs with independently loaded saved extra info), matching versus
unrelated/null input references, actual history-vector destruction and the
actual base-widget destructor with a shown attachment (no hide callback before
its geometry-base endpoint), plus full geometry/base-object destruction with
and without a top-level anchored dependent. The latter removes its anchor and
dependency while leaving its shown flag intact. Registry, parent detach/deleting destructor, local
storage, map/load/default, geometry-base destruction, vector/memory primitives
and script VM operations are captured endpoints. The deleting-destructor endpoint
does not mutate the fixture RB tree; the iterator instructions run unchanged.
Instructions are in `native-chat-reset-source-20261018.txt`. These are offline
oracles with no writes to the original client or its local configuration.

All **86 selected tests** passed (one existing ignored font-metrics test), across
the library, chat lifecycle/context/locking, generic attachments, keyboard,
popups, dragging, resizing and HUD widgets. After removing the unsupported
attachment-hide addition, all **35 affected tests** passed again. Seven reset
tests cover immediate descendant/focus/IME/event retirement, hidden/unconfigured
windows, retained-history clearing without live windows, dead pointer release,
owned versus unrelated move/resize capture, preservation of held items, and the
unchanged original popup/recreation/physical-tab-click lifecycle.

The aa-dev `native_chat_reset` example builds and self-closes after each capture.
The inspected `shots/aa-ui/native-chat-reset-720-20261018.png` shows the recreated
original 430x350 chat at 1280x720/UI scale 0.85, with only fresh messages and no
old popup; it reports **241.2-297.6 FPS**. A separate inspected 1920x1080/scale 1
capture (`native-chat-reset-20261018.png`) reports **250.5-308.0 FPS**. Both log
zero script errors and no runtime warnings/errors. No matching original reset
reference screenshot is available; original-script geometry is preserved.

### Honor and vocation numeric text tokens (2026-10-18)

The original quest-context vocation reward (`questcontext/common.alb`,
CreateQuestRewardLivingPointDisplay.UpdateValue) emits `|l%d` without a final
semicolon. The previous decoder displayed that control sequence literally;
its lowercase semicolon branch also copied numeric bodies without native
canonicalization. `|h`/`|H` and `|l`/`|L` now share the source-closed numeric
parser used by dishonor, accepting semicolon or EOF, canonical signed decimal,
wrapping i64 arithmetic and the native UTF-8-codepoint-to-low-byte quirks.
Full source getters, literal edit text, color runs and links remain intact.

Native `39273e34..52` dispatches both cases of l/L to `392742a7`, and h/H to
`39274250`. Both invoke `3926fbd0`, then `39273190` with icon kinds 6 and 3
respectively. The initialized read-only native table capture already retained
the corresponding `ui/common/money_window.dds` rectangles: honor
**(48,31,14,21)** and vocation **(190,13,17,15)**. These replace the former atlas
estimates. Shared icon advance, scaling/centering, other token families,
dynamic locale-specific rectangle configuration and the generic comma option
remain separate boundaries.

`data/extract/aa-ui/honor_living_text_oracle.py` and
`native-honor-living-text-oracle-20261018.json` execute **181 native groups**
using the actual original parser, number helper, XlGetChar and XlAtoI64.
Both cases of both tokens cover EOF/semicolon, empty and invalid bodies,
canonical signs/zeros, wrapping limits, embedded bars/newlines/NUL, low-byte
Unicode cases, surrounding text and escapes. Memory primitives and final
glyph/value layout are captured endpoints; numeric and consumption behavior
executes unchanged. Atlas evidence remains the separate initialized runtime
capture `native-formatted-text-icon-runtime-20261018.json`.

All **45 selected tests** pass (one existing ignored font-metrics test), covering
the library, formatted numeric text, chat/item/quest links, literal chat input,
direct drawable text and unusable-skill slot styling. The three added tests
exercise native numeric goldens, source icon rectangles/color/link/edit behavior,
and unchanged original character/quest controls with received-style point events.
The fixture opens original quest content and aliases its actual reward widgets;
the reward factory names in the decompiled survey are local closures.

The aa-dev `native_honor_living_text` example builds. Its inspected
`shots/aa-ui/native-honor-living-20261018.png` shows the unchanged original
character panel at 1280x720/UI scale 0.85: fixture source values 001234 and
000567 display as **1234 plus the honor medal** and **567 plus the vocation
badge**. The panel/icon appearance was compared with
`reference/ingame/character_info/real.png` (which has zero points). It reports
zero script errors, no runtime warnings/errors and **229.0-262.1 FPS**, then
self-closes. The quest reward's original missing-semicolon UpdateValue path is
exercised separately in the same example before closing the quest window.

### Plain numeric formatted text (2026-10-18)

`|,number;` now uses the native signed-number parser rather than copying its
body. Like the other numeric tokens, it accepts EOF, canonicalizes signs/zeros,
wraps i64 arithmetic, consumes embedded bars/newlines and follows the native
UTF-8-codepoint low-byte behavior. It emits no icon. The original character
crime-point control, quest rewards, pet/player experience tooltips and labor
displays use this token. Full source getters and literal edit text are retained.

Native dispatch `39273eb2..39273f56` calls `3926fbd0`, actual XL XlI64toA
(`3301bf30`), comma formatter `392701d0`, then ordinary string layout. Original
`north_america.lua` and `russia.lua` explicitly call
`UIParent:SetUseInsertComma(false)`. This slice preserves that ungrouped output.
The generic comma-option hook, its protected-data initial value and existing
rendered-text cache lifetime remain separate source/implementation boundaries;
no global relayout behavior is inferred from the setter's single byte write.

`data/extract/aa-ui/comma_text_oracle.py` and
`native-comma-text-oracle-20261018.json` execute **96 native groups**: actual
token parsing/XL integer conversion with comma formatting enabled and disabled,
and the typed SetUseInsertComma adapter/wrapper. The wrapper accepts bool or
numeric nonzero, rejects nil/string and returns zero Lua values. Allocation,
memory and final string layout are captured endpoints; parser/conversion and
formatting instructions execute unchanged. Production here implements only the
ungrouped decoder, with no additional Lua API or option/cache policy.

All **47 selected tests** pass (one existing ignored font-metrics test), across
the library, formatted numeric text, chat/item/quest links, literal chat input,
drawable text and slot state. Two added tests cover the plain numeric token's
native goldens/no-icon output, literal-edit preservation and unchanged original
crime-point markup through `CRIME_REPORTED` refresh. The existing aa-dev
`native_dishonor_text` example rebuilds successfully for regression capture.

The inspected `shots/aa-ui/native-comma-text-20261018.png` shows original
Crime Points **80 / 50** and Infamy Pts **1234 plus skull** at 1280x720/UI scale
0.85. The unchanged base character panel was compared with the original
character-info reference; its extra crime rows follow the explicit fixture.
The process reports zero script errors, no runtime warnings/errors and
**506.8-560.9 FPS**, then self-closes.

### Pending Trial chat input method (2026-10-18)

Two application-only Rust helpers now match the native chat manager's pending
method state; they add no Lua X2 method:

```rust
aa_ui::lua::chat_input::set_forced_method(lua, method_i32);
aa_ui::lua::chat_input::prepare_input(lua, window_id);
```

`set_forced_method` always stores the value. Except for sentinel **-6**, it
immediately restores the selected chat window's method when the edit's **own**
visibility flag is true, independently of focus or parent visibility. The
pending value remains stored after immediate application. **-6 only clears the
request**; it does not restore or clear the visible input. The application
calls it at its received Trial transition and at chat-manager reload/reset.
The existing `reset_windows` helper keeps its window/history-only contract.

`prepare_input` belongs immediately before the application's actual input-open
Show/prefill/focus sequence. It consumes pending **11** only when that window's
current method differs from 11, then stores -6. Already-Trial input retains the
request; other pending values are not consumed. Generic Show/SetFocus calls do
not apply this policy. No autocomplete, method-list or channel restriction is
inferred from the pending value.

Native setter `39508950` initializes/reloads UIData if necessary, stores signed
method at `3a5bb120` (UIData+80), then checks selected window `39502020`, own
edit visibility `395128e0` (IsVisible(false)), edit `39512960` and virtual+84
before restoration `395107d0`. Native input opening `39512ce0`, specifically
`39512d12..39512d3f`, consumes only 11 before its Show operation at `39512d49`.
Manager reload `395072c0` initializes the pending value to -6. Net source
separately closes Trial173/175 setting 11 before adoption, 17e clearing -6
before role reset, and local180 clearing -6 before closing/resetting context;
the toolkit does not own those protocol transitions.

Restoration also fixes hydration's existing use of the ordinary command setter.
Native `395107d0` clears the prefix, then invokes actual EditBox SetText(empty)
**before writing the new method**. Changed text dispatches OnTextChanged while
the old method/selector is still present. Unchanged empty text takes the native
early return, retaining edit state and skipping the text callback. Method save
through `AAUI.ChatMethod` still precedes selector display/OnContentUpdated.
Non-whisper restoration preserves both the cached whisper target and any prefix
the text callback installs; it does not clear that prefix a second time.
Ordinary typed-command behavior remains separate.

`data/extract/aa-ui/chat_forced_method_oracle.py` and
`native-chat-forced-method-oracle-20261018.json` execute **14 native groups**:
the setter, deferred opening through its pre-show point, restoration, actual
EditBox `39292f70` including XL UTF-8 conversion/UTF-32 comparison, actual
`39510e50` and OnTextChanged dispatch `39292460`. Explicit captured endpoints
are GUI/layout, model string assignment, prefix, GetText/empty command parsing,
Lua event delivery, persistence and selected-window lookup. The oracle covers
immediate/closed/missing-selected inputs, repeat/empty drafts, signed methods,
-6, already-Trial retention, non-Trial pending values, lazy initialization/reload
and callback prefix mutation. No live client is executed or written.

All **74 selected tests** pass across forced method (nine), existing method,
chat lifecycle/reset/links/context/locks/transport, selection, IME and keyboard.
Coverage includes the old-method callback/save/display order, unchanged empty
edit state, hidden-parent own visibility, unrelated focus, deferred consumption,
cached whisper retention, hydration, composition cancellation, retired/invalid
window safety and both actual-original strict pointer/typing paths. The new
`native_chat_forced_method` aa-dev example builds successfully.

Inspected final captures show the original Trial selector and physically typed
`Ready to speak in Trial`, with the old draft absent:

* `shots/aa-ui/native-chat-forced-deferred-final-720-20261018.png`,
  1280x720/default UI scale0.85: **322.9-551.2 FPS**.
* `shots/aa-ui/native-chat-forced-immediate-final-1080-20261018.png`,
  1920x1080/UI scale1: **445.4-494.3 FPS**.

Both report zero script errors, no runtime warnings/errors and self-close.
The original chat/HUD artwork was compared with the character-info reference;
the centered single-tab placement and test text are explicit fixture settings.

### Native task admission and visual-failure audit (2026-10-18)

`RequestTaskDialog` is an application compatibility helper, not an exported
native Lua method. Native X2DialogManager exports RequestDefaultDialog and
RequestNoticeDialog; their wrappers `3905ed20`/`3905edc0` return **one lowercase,
unpadded hexadecimal string handle**, including **"0"** on failure, through
`39424860`/XL conversion. The toolkit's widget-valued compatibility return is
retained separately from that native handle contract.

Generic task admission `3902d570` has distinct outcomes:

* Raw environment flags +a8=true/+a9=false or an identical pending **task
  pointer** return silently, without choice callback or destruction. This is
  not a general duplicate-type, owner, modal-stack or active-dialog limit.
* Derived CanStart virtual+10=false calls choice virtual+18 with
  `buttonType == 1`, then destroys the incoming task. A notice therefore gets
  true; OK/Cancel and custom get false. Derived type/creator rules remain
  task-specific. Trial7's creator prechecks type7 before allocation; Trial6/8
  reject newly allocated duplicates through their own CanStart.
* Accepted type5 while Beautyshop is active calls custom virtual+14 with index2
  and destroys the incoming task. CharacterCustomizer singleton `3931e2f0`
  +480 is set/cleared by `3931ef40`/`3931f0a0`; both emit TOGGLE_BEAUTYSHOP
  (event0x244) with that bool. Registry `3921657c..86` and FireUIEvent's
  `39d22ce8` lookup close the event identity. This is not ordinary modal policy.
* Otherwise `3902d400` attempts a visual, then the task is appended to the
  pending list **even if visual creation failed**. Task+4 records visual
  success. Update `3902d690` retries false rows without rerunning CanStart.

Visual creation `3902d270` fails before widget allocation when raw global
`3a5a4db8` is set or the default decorator is missing. A failed widget allocation
or unreadable mandatory info also returns false. With a default decorator,
absence of a type-specific decorator is allowed. Successful order is default
decorator, optional specific decorator, Show, handle/task map insertion, then
output handle. These raw engine gates are not assigned guessed app semantics.

Removal `3902cef0` reaches `3902ce30` with skip-choice=false: invoke the same
`buttonType == 1` choice while the task is still queued, close its visual,
destroy the task, unlink/free the node and decrement count. `3902cf10` uses
skip-choice=true. Native task cancellation is therefore not silent queue
invalidation; visual creation failure is not admission rejection either.

Read-only evidence is under `data/extract/aa-ui/`: `task_admission_oracle.py`
and `native-task-admission-oracle-20261018.json` execute **21 native groups**
across admission/list insertion, retry, actual removal and visual gates/order.
Synthetic virtual providers, allocation/free, GUI/table/map primitives and
delivery endpoints are captured explicitly. `task_handle_return_oracle.py`
and its JSON execute **six** actual wrapper/XL conversion groups, capturing
owner conversion, manager/create result and Lua return interface. The combined
disassembly is `native-task-admission-source-20261018.txt`. No original or live
client files/processes are modified.

The compatibility helper now returns nil before allocating any widget when
the default decorator is unavailable. Typed application task owners retain
and retry the request rather than treating nil as a choice. The toolkit adds
no inferred global/editor/type/modal gates or duplicate suppression. All
**21 selected tests** pass across popup lifecycle (nine), attachment (five)
and full chat reset (seven). The two new tests verify no allocation/callback
before decoration is available, same-type and missing-specific acceptance,
retry after actual original-stage registration and a strict physical click on
the original invitation's Decline button (custom index3).

### Native cooldown-button and slot captions (2026-10-18)

Generic cooldown buttons do not have the slot's time caption. Native
SetCoolDown wrapper `392322d0` reaches `392809b0`, storing total/remain at
+5a8/+5ac. Drawing `392815f0` calls mask `39280ad0` and ordinary button text
`39280de0`; it never calls slot caption `392c50f0`. Factory cases20..23 at
`392a614c..61fe` construct the four cooldown button variants. Their constructors
install vtables `399c5fb0`, `399c6170`, `399c6330`, `399c64f0`; each +154 points
to `392815f0`. The toolkit therefore removes generated numbers from all four
classes while retaining their masks, ordinary text, callbacks, icon identity
and clicks. This includes original equipped-item constant buttons.

Ordinary slot draw `392c67e0` skips mask and caption when total is below
**200f32 ms**; equality is included. Caption `392c50f0` skips explicit native
+311 global state, respects CVar `slot_cooldown_visible` (`39d3152c`), and permits
captions if that CVar pointer is absent. Zero CVar value hides numbers.
`aa_ui::lua::slot::set_cooldown_numbers(&Lua, bool)` supplies actual option82
(`OPTION_ITEM_SLOT_COOLDOWN_VISIBLE`) without rebinding contents, restarting
timers or invoking content callbacks. The native CVar registration default
is not inferred from this null-pointer fallback.

The formatter multiplies f32 remaining milliseconds by `0.001f32`. Above
3600 seconds it uses `1 - CVTTSS2SI(seconds * -1/3600f32)` hours; above60 it
uses the analogous minutes operation; otherwise it uses truncation plus1
seconds, with native i32 wrapping and CVTT indefinite integer behavior.
There is no day branch. Examples from executed native code:

| Remaining milliseconds | Caption |
|---|---|
| 999 | `1s` |
| 1000 | `2s` |
| 59999.99609375 | `61s` |
| 60000 | `2m` |
| 3599999.75 | `61m` |
| 3600000 | `2h` |
| 86400000 | `25h` |

Category98 `second_initial`, `minute_initial`, `hour_initial` templates are
hydrated through existing `UiRuntime::set_ui_texts`. The unloaded en_us fixture
fallbacks are `$1s`, `$1m`, `$1h`. Native formatting and `$1` replacement
`39426f40` have an eight-byte destination including NUL. Executing the
subsequent text assignment `3922b830`/actual XlMultiToWide confirms that a
partial final UTF-8 character is dropped: controlled `$1éé€` with value9
produces bytes `39c3a9c3a9e282`, then displayed UTF-32 `9éé`. The renderer
preserves that complete-character prefix.

`data/extract/aa-ui/cooldown_label_oracle.py` and
`native-cooldown-label-oracle-20261018.json` execute **37 native groups**:
actual slot draw gate/caption, XL integer formatting, `$1` substitution,
generic mask/ordinary-text path, CVar and explicit-global gates, f32 boundaries,
ASCII byte limits and localized UTF-8 conversion. External memory allocation/
copy, localizer/CVar providers and final mask/font delivery are captured;
no native arithmetic or formatting is replaced. Controlled localization rows
are explicit fixtures; the three stock en_us templates come from read-only
compact.sqlite3. Disassembly is in
`native-cooldown-caption-source-20261018.txt`. No original/live process is
modified.

**Remaining global/own selection boundary:** native slot frame
`392c74ba..77ab` reads the actor manager's separate raw global timer via
`39209430`/`39899ce0`. It chooses that branch only when previous own-active
+312 is false, remaining is nonzero, and accessor kind is not16/17/19. Branch
entry/exit changes +311 and emits global_cooldown_start/done. Otherwise it
calls accessor virtual+18 with final bool0. SkillAccessor `399bb0a0` +18 is
`39209810`: bool0 calls descriptor-aware combined query `3989a3e0`, while
bool1 forces ignoring global via `3989a290`. Thus even the second branch is
not an own-only query. The content adapter still does not expose these
separate inputs/state/events. Duration or the combined winning timer cannot
establish +311, and no third SkillCooldown return or heuristic is introduced.
Mask tessellation, raw setter normalization and that selection lifecycle
remain separate open boundaries.

QA: **18 selected tests pass** across cooldown captions (six), constant-button
lifecycle (five), HUD widgets (six) and original slot state (one). The six
caption tests pass again after the final fixture isolation change. Coverage
includes native float goldens, total200 equality, short ordinary timers,
option changes while active/rebinding, exactly-once expiry, current template
replacement/UTF-8 byte limits, all four generic classes with retained ordinary
button text, strict physical original action-slot use and unchanged original
equipment identity/callbacks. The aa-dev example builds.

Inspected unique final captures at 1280x720/default scale0.85:

* `shots/aa-ui/native-cooldown-caption-on-final-720-20261018.png`:
  original action slots `2s`/`2m`/`25h`, original equipment masks without
  generated numbers, **367.4-410.5 FPS**.
* `shots/aa-ui/native-cooldown-caption-off-final-720-20261018.png`:
  identical icons/masks with slot numbers disabled, **388.9-424.3 FPS**.

Both have zero script/runtime warnings/errors and self-close (PIDs22520/14552).
The copied executable was removed. The original equipment artwork was compared
with the earlier native constant-button capture and HUD reference. This is an
isolated explicit timer fixture, not a claim that native +311 selection is
implemented. Initial diagnostic captures mistakenly rebound every hidden HUD
slot each frame; the final example updates only its three demonstrated slots
and hides unrelated roots after original initialization settles.

### Generic cooldown draw snapshots (2026-10-18)

A generic SetCoolDown call stores drawing state rather than starting another
clock. Setter `392809b0` writes raw f32 total/remain at +5a8/+5ac. Base button
frame virtual+158 (`39280f50`) updates its ordinary repeat-click accumulator;
it does not decrement those drawing fields. The Action/Inventory/Constant
frame overrides are `392816b0`/`39281c20`/`39281df0`. They query providers and
notify the original OnContentUpdated consumer; those notifications do not
independently replace the stored drawing sample. The consumer's subsequent
SetCoolDown call does so.

`data/extract/aa-ui/cooldown_snapshot_oracle.py` and
`native-cooldown-snapshot-oracle-20261018.json` execute **seven groups**:
five raw setter/base-frame cases and two actual Inventory/Constant provider
notification cases. The base frame runs at raw dt values0/16/1000/9000 and
keeps identical stored fields and mask input. The two derived frames receive
explicit500/1000 provider values while an old8000/12000 draw sample is stored;
they emit those new values without mutating the old drawing fields. Executing
the actual setter afterward changes the sample. Provider/descriptor lookups,
final delivery and memory allocation/copy are explicit captured endpoints;
native setter/frame/variant control and mask arithmetic execute unchanged.
Disassembly and field-write scan are in
`native-cooldown-snapshot-source-20261018.txt`. No live process is modified.

The toolkit now retains generic remaining time in `Cooldown.sampled_ms` and
excludes generic snapshots from the local countdown registry. Ordinary slots
continue using end_ms and preserve expiry/events. Original generic provider
queries and original Lua consumer ordering remain intact. A callback that is
removed cannot silently change the drawing sample; reinstating the original
consumer applies its next actual value. Native raw-total normalization and
out-of-range mask/shader behavior remain separate boundaries; this change
does not alter existing setter normalization or infer a GCD classification.

All **20 selected tests pass**: constant cooldown seven, captions six, HUD
widgets six and original slot state one. New coverage verifies all four
generic classes retain their mask past9000 UI milliseconds and clear only
when explicitly set to zero; the actual original constant-button consumer
alone replaces8000/12000 with500/1000, then clears on received readiness.
Existing ordinary slot countdown/exactly-once completion, physical original
action-slot use, equipped-item icon/callback identity and caption option tests
remain green.

The aa-dev `native_cooldown_snapshot` example builds. Its inspected unique
`shots/aa-ui/native-cooldown-snapshot-720-20261018.png` at1280x720/scale0.85
after8seconds shows the original base button still masked from its single
1200/1500 call, while the ordinary first slot has expired and other ordinary
captions have advanced to53s/24h. Original provider-driven equipment remains
visible. It reports **260.5-325.3 FPS**, zero script/runtime warnings/errors,
and self-closes (PID9068); the copied executable was removed. The isolated
panel and explicit sample values are fixture settings.

### Generic cooldown mask texture, UV and selected art (2026-10-18)

Generic cooldown constructors initialize texture+5b0 to null, and draw
`39280ad0` returns immediately without it. A supplied timer alone is not a
mask. Original `SetViewOfUpdateCooldown` in baselib/common_view sets
`ui/icon/cooldown_mask.dds` and pixel coords0/0/256/256 explicitly.

SetCoolDownMask wrapper `39232320` calls loader `3921ad70`. Failure leaves
both old texture and UV intact. Success reaches `392815b0`: release the old
reference, retain the new texture and reset its UV through a zero rectangle.
That reset applies even when the same texture is selected again. Both setters
return no Lua values. Texture coordinates are independent of the timer sample
and survive SetCoolDown(0,0).

SetCoolDownMaskCoords wrapper `392323a0` reaches `392809e0`: compute f32
reciprocals of the texture dimensions, multiply x/y by those reciprocals and
compute right/bottom from x+width/y+height. A zero width or height instead
sets the corresponding normalized endpoint to exactly1. Negative extents
remain reversed; no coordinate clamp is inserted. The native setter dereferences
a missing texture, so the toolkit safely refuses coords-before-mask rather
than claiming a native no-op contract for that invalid call order.

`data/extract/aa-ui/cooldown_mask_oracle.py` and
`native-cooldown-mask-oracle-20261018.json` execute **12 native groups**:
actual wrapper, ownership switch, float normalization and rectangle copy.
Script interface/return, loader, texture dimensions/reference functions are
explicit endpoints. Cases cover fresh/no texture, default/full reset,
subrectangles, zero/negative/fractional extents (including non-power-of-two
provider dimensions), failed-load retention and same-image reset. Native float
results are recorded as exact bit patterns. Disassembly is
`native-cooldown-mask-source-20261018.txt`; no live process is modified.

The original stock mask was extracted only through aa-pak (one87536-byte DDS):
256x256, RGB46/46/46, with alpha199 on49479 pixels and200 on16057 pixels.
`ButtonData.cooldown_mask` retains its resolved path and normalized UV.
Native `39280ad0` obtains the selected button background and accepts only image
or icon-image kinds2/3. Image virtual+104 is `3928b3c0`; Icon's is `3928ed10`.
They call `3928ace0` with BOTH the original image texture/UV and mask texture/UV.
Icons draw each retained texture layer, sharing the same object+d0 image UV.
Original SetIconBackGround uses one-pixel anchors, so a42px equipment button
has a40px mask. Missing/hidden/unsupported selected art yields no mask.

The actual source `game/shaders/hwscripts/cryfx/x2ui.cfx` (2679 bytes, read
through aa-pak) closes the blend. Image color is multiplied by vertex color;
mask RGB becomes `mask.rgb * 2 - 1`; final RGB is the saturated sum of those
values and final alpha is image-alpha times mask-alpha. This preserves the
original image's transparency. It is not a flat mask overlay. The base sampler
clamps and the D3D mask sampler wraps. A narrow Material2d pass implements
those operations, converting Bevy's linear samples/colors to sRGB for the
source UI arithmetic and returning linear output. Unit-square mesh UVs
reconstruct both independent rectangles in the shader, including reversed or
zero image-UV extents; clipping and transforms retain their association.

Native `3928ace0` advances through eight equal time sectors along the image
rectangle perimeter. It linearly interpolates the first remaining edge, then
submits centre/end/start triangles with both UV streams through `39220920`.
`cooldown_fan_oracle.py` / `native-cooldown-fan-oracle-20261018.json` execute
**12 fan groups** with only the final submission endpoint captured. Actual native
rectangle/midpoint/f32 sector/UV arithmetic runs unchanged: zero, half-sector,
sector boundaries,0.2/0.33333334/0.999/1 progress, rectangular geometry and
zero image-UV extents. Four further groups execute the actual downstream
`39220920`/`3921f2a0` and capture the final batch: integer/fractional/negative
positions prove add0.5/CVTT snapping after interpolation, and source color
0.3/0.5/0.7/0.4 yields packed RGBA8 `0x66b27f4c`. The generic pass applies
that packing and keeps the original fractional UVs through vertex snapping.
`native-cooldown-raster-source-20261018.txt` archives
the native path. Generic buttons now use that eight-sector fan. Ordinary slot
tessellation/color and unresolved native GCD selection remain unchanged.
The existing renderer's general pixel snapping/effect transform boundaries
remain; this slice does not claim a wider animation renderer correction.

Existing bare-widget tests now install actual mask/image state before
expecting a sweep. New coverage checks missing-mask/load-failure retention,
raw UV bits, selected art geometry, independent image/mask UV and color,
Icon multi-layer UV reuse, unsupported NinePart art and executed fan goldens.

Fresh `cargo check -p aa-ui` passes. All **38 selected regressions pass**:
mask7, constant cooldown7, caption6, HUD6, slot state1, scale/effect clipping4
and chat clipping7. These retain original provider/consumer callback ordering,
action-slot physical click/identity, snapshot lifetime, caption toggles and
ordinary-slot countdown/completion. Only the generic button's mask pass uses
the new material; text, other artwork and ordinary slot rendering stay on the
existing ColorMaterial path. Material views no longer in the current mesh
list are retired, avoiding retention of every historical UV selection.

The aa-dev `native_cooldown_mask` example builds and uses actual original action
slots/equipment controls plus explicit texture/crop/transparent/no-mask fixture
controls. Final inspected captures are
`shots/aa-ui/native-cooldown-mask-final-720-20261018.png` at1280x720/default0.85
(**274.7-329.0 FPS**) and
`shots/aa-ui/native-cooldown-mask-final-1080-20261018.png` at1920x1080/1.0
(**296.7-317.4 FPS**). Both have zero script errors, shader/runtime warnings or
errors, and self-close (PIDs51428/1608); the copied executable was removed.
The transparency diagnostic uses original `ui/icon/item_pick_cursor.dds`:
32x32,946 alpha-zero pixels,77 alpha255 and one alpha254. Four corresponding
transparent pixel samples at each resolution exactly retain the unmasked
underlying artwork; `native-cooldown-mask-alpha-pixels-20261018.json` records
those results. The initial diagnostic `slot_over.dds` was found to be fully
opaque and was replaced before the final captures. No asset was edited.

### Original bounty bulletin globals are an asset defect (2026-10-18)

The actual original nonempty bounty bulletin fails at
`usertrial/bounty_bullet.alb:115`. This is not a decompiler-renamed local:
actual `main/0/4` pc15 reads `GETGLOBAL listCtrl`, pc20 reads
`GETGLOBAL bountyRecords`, and pc21 reads `GETGLOBAL index`, although its loop
length uses the uppercase `BOUNTY_RECORDS` upvalue. The original Bounty button
similarly reads bare lowercase `curSelected`/`bountyRecords` at lines53/54.

A bytecode scan covers all **961 original .alb scripts**, matching the pak
index count. No script executes SETGLOBAL for `listCtrl`, `bountyRecords` or
`curSelected`. The lowercase data/selection reads occur only in the bounty
asset. There ARE four unrelated SETGLOBAL writes to `index` in housing
maintain-window and tooltip functions, so `index` is not universally absent;
those writes do not supply the missing bounty table, selected row or list.
`native-bounty-global-script-refs-20261018.json` records every relevant opcode,
the full scan counts, bounty asset hash and native ASCII-string presence.
The x2game PE contains no `bountyRecords`, `curSelected` or `listCtrl` strings;
`index` is present and is not used as evidence of an absent native binding.

`components/pagelistctrl.alb:28` creates its local list with
`frame:CreateChildWidget('listctrl','listCtrl',0,true)`; line33 writes
`SETTABLE frame.listCtrl`, rather than a global. Native wrapper `39266960`
reaches `39279cb0`; its zero-index attachment at `39279eb6` calls `3920d330`
with the PARENT script table. The executed `bounty_child_binding_oracle.py` /
`native-bounty-child-binding-oracle-20261018.json` runs actual `3920d330` and
captures only script-table/refcount endpoints: AddRef child, SetAny on the
parent receiver with key `listCtrl`/type6/child object, then Release. It does
not export a bare global. Source and exact bytecode artifacts are
`native-bounty-child-binding-source-20261018.txt`,
`native-bounty-fill-bytecode-20261018.txt`,
`native-bounty-button-bytecode-20261018.txt` and
`native-bounty-pagelist-bytecode-20261018.txt` under data/extract/aa-ui.

The usertrial toc explicitly loads bounty_bullet, whose original main
registers TOGGLE_BOUNTY_BULLETIN. This evidence supports a broken nonempty
asset path; it does not establish that the screen was unused. No toolkit
globals, bytecode edits, replacement rows or optimistic bounty actions are
introduced to conceal it. The client integration documents the boundary.

### Native ordinary-slot global/accessor selection (2026-10-18)

Native slot frame `392c7360` owns two independent flags: selected-global+311
and prior successful accessor-remaining-nonzero+312. Constructor
`392c79d5..79ec` zeros both flags and both f32 timer fields. The raw global
getter `39209430` reads the local actor manager through `39899ce0`, copying
its u32 remaining+8/total+c. This is separate from the accessor's combined
winning timer. The received ActionSlot kind is a different domain from the
native accessor field+4; the adapter requires the actual accessor kind.

When prior accessor-active is false, raw global remaining is nonzero, and
accessor kind is not16/17/19, the frame chooses global. It sets+311 before
`global_cooldown_start(total)` and DOES NOT query the accessor. Repeated
global frames do not repeat that event. On leaving this branch, it clears
+311 and emits `global_cooldown_done` BEFORE re-reading/querying the current
accessor. Consequently that callback can change the subsequent binding/data.
The accessor query receives false for its ignore-global argument; its result
can itself include global cooldown. No duration comparison classifies it.

A successful accessor query stores both counts narrowed individually to f32.
The remaining-zero transition controls+312 and `skill_cooldown_start(total)` /
`skill_cooldown_done`, with flags set before their callbacks. A false query
silently zeros both timers/flags without a skill-done event. A previously
active accessor bypasses the global selector even when a fresh raw global
count exists. Native `ResetState` (`39256300` -> `392c2b40`) clears both flags
while retaining timer fields, without cooldown events. Its native accessor
cache reset and completion-child effect remain outside this plain UI adapter.

The public app-only API is `aa_ui::lua::slot::{CooldownSample,
set_native_cooldown_mode, update_native_slot_cooldown}`. Mode defaults off,
retaining the existing content-hook countdown path. Explicitly enabling it
before control binding bypasses content cooldown estimates for ordinary slots.
The app then supplies raw u32 global counts and native accessor kind per live
slot, plus an on-demand `FnOnce(&Lua,Id)->Option<CooldownSample>` accessor
query. Some zero/zero is a successful ready result; None is native query
failure. The caller skips absent accessors. The helper executes that query
only after the selector and any global-done callback. Raw samples remain
snapshots: advancing toolkit time alone cannot finish them. Changing adapter
mode clears UI samples/flags without modifying game timers; repeated identical
mode installation is a no-op. Generic cooldown buttons stay independent.

A selected-global snapshot suppresses the ordinary slot caption using+311;
option82 and existing caption formatting still control other snapshots.
There is no new Lua game method/global. Production activation remains gated
on an actual raw-GCD/accessor producer supplied by the app. The existing
`AAUI.SkillCooldown` two-result combined winner cannot substitute for it.
Raw manager insertion, A74/planner provenance and accessor16/17/19 class
mapping are not inferred by this toolkit slice. Ordinary-slot tessellation
and completion-effect rendering retain their existing boundaries.

`data/extract/aa-ui/slot_gcd_selector_oracle.py` and
`native-slot-gcd-selector-oracle-20261018.json` execute **14 native groups**
through actual `392c7360` / `392c2b40`. Raw getter/manager reads, unsigned
conversions, flag transitions and ordinary timer setter execute unchanged;
accessor-frame/query and final script/effect delivery are explicit endpoints.
Goldens cover query-skipping, repeated global, callback order, retained
accessor state, both completion routes, false-query silence, all three
excluded kinds, ResetState, u32-max narrowing and absent accessor retention.
`native-slot-gcd-selection-source-20261018.txt` and
`native-slot-gcd-manager-source-20261018.txt` archive the source paths.

Fresh `cargo check -p aa-ui` passes; **27 selected tests pass**: native
selector6, constant7, caption6, mask7 and slot-state1. They include a
global-done handler that rebinds before the query, native snapshot/ResetState
lifetime, explicit compatibility-mode transitions, original action-slot
content events/captions and a strict physical click on the original control.
The aa-dev `native_slot_cooldown` example uses the actual original action
slots/equipment controls with explicitly fixed samples. Inspected capture
`shots/aa-ui/native-slot-cooldown-720-20261018.png` at1280x720/default0.85
shows global without a caption, retained/excluded accessor snapshots with9s
captions, and independent generic equipment. It reports **404.1-457.3 FPS**,
zero script errors/WARN/ERROR, self-closes (PID48992), and its copied
executable was removed. These fixture samples are not received game state.

### Ordinary slot masks use the native texture fan (2026-10-18)

Native ordinary-slot load `392c2f50` loads stock
`ui/icon/cooldown_mask.dds`, retains it at+314, creates the separate+328 Image
FROM that texture with pixel rectangle0/0/256/256, and anchors that image to
the slot at top-left+1/+1 and bottom-right-1/-1. Its mask geometry is independent
of the bound icon's texture, crop, color or later anchors. Image constructor
`392907b0` copies white RGBA from `3921a930`/39d29254; UI initialization
`3921e081..e0c8` writes four1.0 components there.

Ordinary draw `392c3960` computes f32 progress `(total-remaining)/total`
(default reverse flag+310 is false), then calls Image virtual+104
`3928b3c0` -> `3928ace0`. The slot passes its stock texture as the mask, and
that Image supplies the SAME stock texture as the base input. Thus the actual
x2ui.cfx blend samples the original mask twice. Its RGB46 drives the final
RGB to black, and alpha is the product of both sampled alphas, multiplied by
UI alpha. The former flat RGB46/alpha199 overlay did not match this contract.
The existing dual-texture material now serves ordinary slots as well as
previously corrected generic buttons; no shader or app hook is added here.

The fan advances linearly through eight equal perimeter time sectors. A
40x40 image with1/16 elapsed starts the remaining edge at30/0 relative to
its origin; a trigonometric angle produces a different point. Fractional
rectangle coordinates are interpolated first and only then snapped by native
add0.5/CVTT, retaining the original UVs. The slot uses full independent
image/mask UV rectangles. The shared fan/submission helper preserves the
existing generic Icon multi-layer UV behavior and packed-RGBA8 submission.

Native drawing requires total>=200, a loaded mask, positive remaining and
0<progress<=1. Zero/invalid progress clears both native UI draw timer fields
BEFORE caption rendering, without changing either selector flag or emitting
cooldown events. Explicit native-adapter snapshots implement this lifetime;
a later genuine query sample restores their draw fields. Missing stock mask,
zero remaining or a total below200 do not enter that reset branch. The legacy
content-hook path still owns its estimated local clock, including initially
equal remaining/total counts, and retains that compatibility clock/caption
lifetime rather than erasing the estimate at first paint. Both paths now draw
the native texture fan only for positive valid progress. Reverse+310 producer,
rotating spinner and completion-child effects remain open; no guessed reverse
method or replacement effect is introduced.

`data/extract/aa-ui/slot_mask_oracle.py` and
`native-slot-mask-oracle-20261018.json` execute **12 actual native groups**:
stock load/UV/anchors and failed load, zero/half-sector/sector-boundary/half/
0.2 progress, zero remaining, over-total reset, missing mask, controlled reverse
flag and fractional rectangular geometry. Actual392c2f50 runs to313c; generic
base initialization, texture loader/reference/dimensions, image creation and
anchor endpoints are captured. Actual392c3960 ->3928b3c0 ->3928ace0 executes
unchanged; its controlled draw rectangle, final submission and decorative
effect delivery are explicit endpoints. Both texture identities, all triangle
positions, image/mask UV streams, color and resulting timer fields are recorded.
`native-slot-mask-source-20261018.txt` archives loader/draw/constructor paths.
The controlled reverse case documents the branch without enabling its unknown
producer in the toolkit.

Fresh crate check and **42 selected tests pass**: ordinary-mask5, HUD6,
selector6, caption6, generic-mask7, constant7, slot-state1 and scale/effect4.
Coverage includes exact native sector/UV/texture goldens, fractional rectangle
snapping, independent icon artwork, reset-without-callback lifetime, missing
mask/minimum-total boundaries and compatibility countdown. Existing original
provider/consumer order, caption options, physical original action-slot click,
identity and completion tests remain passing. The earlier trigonometric
10%-remaining area assertion is corrected from145.3 to native160 square px.

The expanded aa-dev `native_slot_cooldown` fixture retains original action
slots/equipment and adds1/16,1/5,1/2 elapsed plus rectangular mask examples.
Both final captures were inspected:
`shots/aa-ui/native-slot-mask-720-20261018.png` at1280x720/default0.85
(**292.6-323.1 FPS**) and
`shots/aa-ui/native-slot-mask-1080-20261018.png` at1920x1080/1.0
(**257.2-306.5 FPS**). Both report zero script errors/WARN/ERROR and self-close
(PIDs4340/50356); the copied executable was removed. The original combat
reference `reference/ingame/combat_triple_slash/real_05.png` was also inspected
for toolbar context; its live cooldown phase is not treated as a pixel golden.
Six paired corresponding pixels across the two fixture resolutions match
rounded black blending by `1-(199/255)^2` exactly in every RGB channel, e.g.
85/38/9 masked versus217/97/22 uncovered. The unmodified captures and pixel
coordinates/results are recorded in
`data/extract/aa-ui/native-slot-mask-pixels-20261018.json`. No art or original
Lua script is modified, and no reverse/GCD producer is guessed.

### Native slot accessor kind is a factory result (2026-10-18)

`slot_accessor_oracle.py` / `native-slot-accessor-oracle-20261018.json` now
record **27 executed factory/prefix/query/default groups**. The factory's
accessor+4 domain must not be confused with either UI ISLOT values or received
ActionSlot kinds. With an ordinary descriptor, actual SkillFactory392c5790
selects SkillAccessor kinds as follows:

| UI source kind | Native accessor kind |
|---|---:|
| ability view243, active skill | 18 |
| mode action246, skill branch | 4 |
| constant249, direct EstablishSkill | 5 |
| action254, skill branch | 12 |

A descriptor tag matching runtime39ac6f08 (closed tag204) takes the earlier
SkillAccessor15 branch BEFORE that UI switch. The oracle confirms UI244 with
tag204 succeeds as15 while ordinary UI244 is rejected by this skill factory.
This is not permission to classify all killstreak or action controls as15.
The actual received/content producer and descriptor tag membership matter.

The three global-selector exclusions are also distinct concrete accessors:

- **16: direct item-template preview.** Registered EstablishItem39256370 calls
  392c6d40 ->392c6100 ->ItemAccessor constructor3920caa0(16), VT399bb388.
  Its actual Update path logs `X2::GameClient::ItemAccessor::Update`.
- **17: callback/icon accessor.** Factory392c5e30 constructs39209900(17),
  VT399bb1a8, for pet-command247 and the script branch of mode-action246.
  It retains a callback pointer and icon/description strings. Its cooldown
  function39209960 returns true without writing the initially-zero outputs.
- **19: BuffAccessor.** Ability-view243 producer types2/3 call392c6020/5f40,
  both constructing392091c0(19), VT399bb160. Its Update3920a650 logs
  `X2::GameClient::BuffAccessor::Update`. Cooldown39209210 likewise returns
  true with untouched zero outputs. Active ability-view skills instead use18.

Direct item16's final query result needs particular care. Actual39209460
calls394df5b0 ->39391a30 and CAN receive nonzero combined use-skill counts.
It then requires a live instance through virtual+20, actual39209610. That
switch has no16 branch and returns null, so the FINAL query bool is false,
even with8000/12000 output counts. Native slot frame392c7360 discards false
query outputs and zeros both draw fields. Thus the app adapter result for
this direct preview is None; callback17 and Buff19 are Some(0/0). A control
ItemAccessor10 with a live instance returns true with the same counts;
missing instance or failed combined timer is false. Exclusion alone was not
used to infer those return values.

Item descriptor+98 is `items.use_skill_id`: SQL39a00d90 column43 is stored at
local record+98 (396cb951..966), then copied into the descriptor. The helper
resolves that skill and reaches existing combined getter393919f0. Live item
queries also have instance/implementation8/9 overrides; a use-skill lookup
alone is not a complete live-item adapter. No guessed item timer is wired.

The executed constructor prefix392c7950 overwrites deliberately nonzero test
fields with zero reverse+310, selected-global+311, accessor-active+312 and
both f32 durations. Generic base construction is an explicit endpoint, and
execution stops after those native stores. The reverse branch remains
unexposed pending an actual setter/producer; its existence does not establish
a Lua method or original runtime user of it.

The oracle executes real skill descriptor/tag-vector/byte-switch branches
and39208ea0 constructor, with catalog lookup/allocation controlled. Item
construction stops after actual kind/vtable/default stores and before live
inventory registration; callback/Buff constructors execute fully before the
factory's descriptor/visual continuation. Separate query cases execute
39209460/394df5b0/39391a30 and actual instance-kind switch39209610, with
catalog/item lookup and combined timer endpoints explicit. Raw counts and
final bool are recorded separately. `native-slot-accessor-source-20261018.txt`
archives these paths, registrations, SQL and descriptor field copies. There
are no production edits, new Lua hooks, network assumptions or mode activation
in this source-only follow-up.

### Native action configuration and reverse-writer follow-up (2026-10-18)

The same accessor oracle now executes **35 groups**, adding actual action
factory392c61e0 with native configuration getter endpoints. These configuration
values are a THIRD domain, distinct from UI ISLOT values and accessor+4. This
audit does not establish a conversion from the received wire ActionKind enum.

| Native configuration kind | Getter/value | Resulting accessor kind |
|---|---|---:|
| 1 | 390898d0, item template | ItemAccessor10 |
| 2 | 39089930, skill ID | SkillAccessor12; matching tag204 selects15 |
| 4 | 39089900, item DBID | ItemAccessor14 if39209380 finds the instance, otherwise11 |
| 5 | 39089960, mate skill ID | MateSkillAccessor13 |
| 0 or3 | no supported branch | no allocation |

The actual native switch and constructors execute unchanged. Configuration
getters, allocation/catalog endpoints, alternate-bar CVar value0 and the
special-mode predicate false are controlled inputs. Every added case asserts
that the switch consumed its configured kind and reached the expected native
constructor continuation; a zero-initialized result alone is not treated as
proof of dispatch. Alternate-bar indexing and special-mode item restrictions
are outside these goldens. Pet-action UI248 separately uses MateSkillAccessor6;
neither that UI value nor the action configuration kind proves its cooldown
query semantics. The source archive now includes the native action factory.

The existing app-only selector API needs no additional Lua hook for these
identities. Its caller still needs genuine raw GCD counts, the actual bound
accessor class and a source-matching query. The query closure must reread the
current binding AFTER global_cooldown_done, because the callback can replace
it. Missing actor/global-manager input requires skipping the update, rather
than fabricating ready0/0. Live item accessors require their instance and
implementation-specific overrides; a combined use-skill timer alone is
insufficient. Direct item16 returns failure, while callback17/Buff19 return
successful zero outputs as documented above. None of this activates native
mode in production or replaces unresolved raw GCD/timeline producers.

`slot_reverse_writer_audit.py` and
`native-slot-reverse-writer-audit-20261018.json` record a bounded follow-up:
11 direct-displacement MOV store candidates for+310, four reverse-like ASCII
strings, and three actual native slot cast-thunk executions. The apparent Lua
writers39247030 and3926ac90 belong to SetUseDoubleClick and SetStickedHeight.
Their required virtual casts+40/+70 execute on the actual slot vtable and
return null; the slot's own cast+a4 returns its receiver. They therefore do
not establish slot reverse setters. Constructor392c79e5's zero word store
and mask draw392c398c's read remain the identified slot writes/read contract.

This is an immediate-displacement store/name scan, not alias analysis or proof
that no runtime writer exists. Alias stores, copies or an external producer
remain possible. No runtime setter has been identified, and no guessed Lua
reverse method is exposed. `native-slot-reverse-writer-source-20261018.txt`
archives the candidate setter registrations, casts, constructor and draw read.
Both source-only scripts were rerun successfully; no Cargo build, visual
change, app adapter or production mode change is part of this follow-up.

### Native ordinary slot cooldown edge artwork (2026-10-18)

Ordinary slots now draw the missing native rotating edge after their mask fan
and before the time caption. Actual loader392c2f50 creates a centered5x22
EffectDrawable from `ui/common/hud.dds` crop748/166/5/22. Its white constant
color phase and internal circle arguments0.45454547/2 produce a ten-pixel
orbit. This fixed-size child stays centered when its slot becomes rectangular;
the bound icon's anchors, crop, color and extent do not control it.

Actual392c3960 sets BOTH motion angle bounds to f32(progress*360), restarts
the effect through3928fb50 and immediately draws3928f810. Restart resets the
phase time on every valid draw, so the edge follows the supplied sweep sample
without an independent clock. The toolkit emits the equivalent stock-art
quad directly. It shares the mask's loaded-texture/positive-remaining/valid-
progress gates and minimum200 total gate; repeated snapshots, invalid-progress
reset, ready state and later restored samples add no callbacks or timer state.
Global caption suppression and generic cooldown-button drawing are unchanged.
Completion-child flashes, reverse producers and raw GCD selection inputs remain
separate unresolved work.

Native Image39220590 applies the effect matrix BEFORE add0.5/CVTT snapping.
The implementation retains the native f32 matrix operation order: reassociating
the transform around local coordinates changes half-pixel results. At1/16
elapsed in a42px slot, final vertices are27/1,31/3,23/23,18/21; at half elapsed
they are24/42,19/42,19/20,24/20. The crop uses its locale-resolved atlas and
white packed-RGBA8 modulation. Existing widget transforms and clipping still
apply through the ordinary renderer.

`slot_edge_motion_oracle.py` /
`native-slot-edge-motion-oracle-20261018.json` execute **31 groups**: actual
loader through328b; native circle/ellipse controls; actual Lua wrapper; complete
effect restart, draw, matrix helpers and Image transform/snapping; fractional
UI scale0.85; rectangular slots; repeated sample, invalid, restore and zero
sequences. Texture allocation/catalog lookup, renderer submission after vertex
arithmetic, frame-time input and external SSE sinf/cosf are explicit endpoints.
Completion drawing is captured separately, without substituting an effect.
`native-slot-edge-motion-source-20261018.txt` archives the relevant bodies.

The internal circle helper's radius order differs from the Lua API. Actual
wrapper392353b0 swaps the radius arguments before calling39291b30; its executed
control confirms that the existing Lua-facing motion implementation and original
autoattack arrow remain correct. No motion API or existing motion test is
changed. Fresh crate check and **55 selected tests pass**, including animation10,
constant7, caption6, generic-mask7, HUD6, scale/clip4, ordinary-mask8, selector6
and slot-state1. New assertions cover actual native vertices/UVs, draw order,
scale, rectangular extent and snapshot/visibility lifetime; original control
provider/consumer order and physical click regressions remain passing.

The aa-dev original-control fixture was built fresh and both captures inspected:
`shots/aa-ui/native-slot-edge-720-20261018.png` at1280x720/default0.85
(**302.5-337.7 FPS**) and
`shots/aa-ui/native-slot-edge-1080-20261018.png` at1920x1080/1.0
(**295.0-366.9 FPS**). Both report zero script errors/WARN/ERROR and self-close
(PIDs21144/41272); the copied executable was removed. The original combat
reference `reference/ingame/combat_triple_slash/real_05.png` was inspected again
for toolbar context, with its unknown live phase kept separate from native
geometry goldens. No original scripts, source assets or app providers change.

### Ordinary slot completion-child draw lifetime (2026-10-18)

The slot's completion child now uses its original stock nine-part art and a
private native phase machine. Loader392c32f3..3612 selects
`ui/common/hud.dds` crop743/35/13/16, insets6/7/6/8 and slot anchors+1/-1.
Phase0 moves white alpha0->1 and scale0.3->1 with velocity/acceleration
times0.5/0.4 seconds. Phase1 keeps scale1 and moves alpha0.9->0 with
times0.3/0.2 seconds; repeat count is1. No substitute art is introduced.

Actual392c7360 clears the accessor-active flag, delivers
skill_cooldown_done to Lua, then calls SetStartEffect(true). Failed queries
and global-done do not start this child; ResetState leaves it intact.
The toolkit follows that order. Its existing compatibility countdown still
removes its timer and delivers its existing callback before starting the same
visual child; the child's retained total only supplies the native draw gate
after that compatibility timer has gone. A current provider timer takes
precedence, including changes made by the callback. No cooldown manager,
query result, selector flag or estimated countdown is replaced by this effect.

The phase machine advances only during actual paint-order traversal, BEFORE
the renderer's alpha skip. Native3927b010/392a8830 controls establish the
admission boundary: hidden ancestors and omitted manager layers do not draw;
fully clipped, offscreen, fading and alpha-zero controls still run the child.
The normal renderer's paint traversal supplies membership; completion does
not invent its own layer/viewport policy. An active mask or total below200
pauses the completion child. Slot ticking only requests repaint and never
advances this clock, so later fade/OnUpdate visibility changes and orphaned
subtrees are respected by the final paint traversal.

Native3928e6c0 advances at most one phase per draw and retains overshoot.
At100ms draw intervals its alphas are0.2125,0.45,0.7125,1,0.9,0, with the
next phase's initial value receiving its own draw. It is not interchangeable
with the existing generic analytic animation sampler. That sampler and every
Lua motion/effect method are unchanged. Final nine-part transforms, pixel
snapping and packed colors follow the native image path.

`slot_completion_oracle.py` /
`native-slot-completion-oracle-20261018.json` execute **36 actual groups**:
full loader/configuration, phase stepping/restart/overshoot, native UV/slice/
matrix/snap/color arithmetic, own/global/false-query/reset triggers, callback
timer mutation, short totals, mask pause, hidden/fading/transparent/clipped/
offscreen/nested admission and manager layer inclusion/exclusion. Texture
allocation, frame-time interface and final renderer submission are explicit
endpoints; the atlas dimensions come from the original DDS extracted through
aa-pak. `native-slot-completion-source-20261018.txt` archives the source paths.

Fresh crate check and example build pass. **92 selected tests pass** across
completion7, ordinary-mask8, native-selector6, slot-state1, animation10,
attachment5, constant7, caption6, generic-mask7, drag12, HUD6, keyboard4,
popup-lifecycle9 and scale/clip4. The completion goldens include the actual
nine-part vertices/UVs, phase handoff/overshoot, callback-before-restart,
callback-mutated total, native mask/minimum-total admission, hidden ancestors,
alpha0, empty clipping, offscreen placement and orphaned paint membership.
Two older test helpers now discriminate the mask texture or edge UV rectangle,
so genuine completion quads do not masquerade as those independent effects.

The original CreateActionSlot/equipment aa-dev fixture was built fresh and all
four captures inspected: `native-slot-completion-active-720-20261018.png`
(**293.0-353.6 FPS**), `native-slot-completion-paused-720-20261018.png`
(**259.0-321.6 FPS**), `native-slot-completion-paused-resumed-1080-20261018.png`
(**301.0-351.7 FPS**) and `native-slot-completion-completed-720-20261018.png`
(**262.9-301.8 FPS**), under `shots/aa-ui/`. The 720p captures use default0.85
scale and 1080p uses1.0. The paused slot is hidden; after reopening, its saved
phase resumes. All four report zero script errors/WARN/ERROR and self-close
(PIDs33776/42180/9128/40704); the copied executable was removed. No source asset,
original script or app producer changes, and native selector activation still
requires the genuine raw GCD/accessor inputs documented above.

### Generic SetCoolDown raw Lua contract (2026-10-18)

The generic setter now follows the complete Lua-facing chain: registration
39232d26..3d selects typed thunk392328b0, parser3922f380 requires two float
values, binding392322d0 resolves/casts the widget, and setter392809b0 copies
raw total/remain to+5a8/+5ac. Actual Cry GetParam3550a460 and conversion3550bf90
accept actual Lua numbers for the requested float type, narrow double to f32,
and reject strings, booleans, tables, nil or missing arguments. Invalid input
leaves the old sample intact; extra arguments are ignored; every call returns
zero results. The toolkit no longer defaults the second argument, parses
numeric strings, clamps total to remaining or deletes nonpositive samples.
Ordinary slots retain their separately documented compatibility clock.

Native draw39280ad0 checks remaining against zero, including its unordered
comparison behavior, and computes raw f32 `(total-remain)/total` without a
positive-total guard. The common fan3928ace0 starts with its carried endpoint
at the center: negative or unordered progress skips interpolation for the first
triangle, then retains each previously emitted perimeter endpoint. This yields
one degenerate triangle plus seven finite perimeter triangles, rather than
clamping progress to zero. Progress>=1, including positive infinity, emits
nothing. Zero-total and nonfinite raw timer values therefore stay stored while
the original arithmetic produces finite position/UV inputs; no nonfinite mesh
coordinates or invented refusal are needed. Valid progress geometry, masks,
captions, callbacks, native selector flags and GCD producers are unchanged.

`cooldown_setter_wrapper_oracle.py` /
`native-cooldown-setter-wrapper-oracle-20261018.json` execute **20 full typed
setter cases plus five raw fan controls**. The actual x2game thunk/parser/
binding/setter and Cry parameter conversion run unchanged; Lua stack top/type/
number primitives, widget lookup/cast, error logging and zero-result delivery
are explicit endpoints. The Lua number endpoint uses FLD qword followed by the
original FSTP f32, preserving source narrowing. Cases cover invalid types,
arity, failed cast, extra args, zero/negative/over-total counts, signed zero,
f32 boundaries, infinity/NaN, raw draw progress and exact finite position/UV bit
patterns. `native-cooldown-setter-wrapper-source-20261018.txt` archives native
instructions and the Cry conversion source. No live process or original asset
is modified.

Fresh crate check and fixture build pass, and **64 selected tests pass**:
generic-mask10, constant7, caption6, ordinary-mask8, native-selector6,
completion7, HUD6, animation10 and scale/clip4. New tests cover all four generic
classes, strict types/arity, invalid-call retention, signed zero, f32 narrowing,
raw values across ticks/draws and finite nonfinite-input fan geometry. The two
original constant-button consumer assertions now check their actual explicit
SetCoolDown(0,0) raw sample, preserving the separate icon/click/callback checks.

The aa-dev original-button fixture calls original SetViewOfUpdateCooldown before
its explicit samples. Final captures were inspected at
`shots/aa-ui/native-raw-cooldown-final-720-20261018.png`
(1280x720/default0.85, **347.3-400.0 FPS**) and
`shots/aa-ui/native-raw-cooldown-final-1080-20261018.png`
(1920x1080/1.0, **328.4-399.4 FPS**). They demonstrate retained samples after6
and9 seconds, over-total/zero-total masks, rejected-string retention and
negative-remaining suppression. Both report zero script errors/WARN/ERROR and
self-close (PIDs27116/50896); the copied executable was removed. Initial
diagnostic captures omitted that original mask setup and are not visual mask
evidence. Original source scripts/assets and app cooldown providers are unchanged.

### Item cooldown query ordering and clock inputs (2026-10-18 source audit)

Actual ItemAccessor39209460 first selects the template from native kind10/16's
field or a resolved live instance, then calls394df5b0->39391a30->393919f0.
SQL loader396cb3f0 selects items.impl_id at column21 into temporary+48
(396cb6f5), and items.use_skill_id at column43 into temporary+98
(396cb966); actual copy396ba730 preserves those fields at+48/+98. The ordinary
query requires a successful combined use-skill result BEFORE its live-instance
and implementation overrides. Missing base results zero the outputs and return
false. A later missing instance returns false with prior outputs still present;
the caller must honor the boolean. Direct kind16 remains false as established
by the factory audit, even when its intermediate use-skill query succeeds.

Generic Inventory394e2910 resolves its packed source first and applies the same
implementation-specific overrides BEFORE falling back to the use-skill query.
It therefore cannot stand in for the ordinary accessor query. Implementation8
reads raw instance byte+1d: nonzero returns600000/600000. Otherwise it compares
the separate seconds clock with raw instance u64+1e. At differences<=600 seconds,
remaining is `u32(trunc_i64(600-difference))*1000` with wrapping multiplication,
total600000. A difference>600 falls back. A timestamp one second in the future
thus produces601000/600000; this is a real raw over-total input.

The seconds interface is Cry Timer3660c060 virtual+6c->3656eac0: actual
XlGetCurrentFileTime3301ca70 is time64 seconds, plus the separate timer offset
set by virtual+68->3656ea90. XlDiffTime3301ca80 calls difftime64. The name
FileTime here does not mean Win32 100ns FILETIME ticks. Implementation9 helper
393a6d90 instead uses raw LOCAL timer0 milliseconds (virtual+10->3656f740,
low32), without the TimeKeeper/server mapper. Its shared deadline3a17a950
overrides outputs to(deadline-now,2000) only when the deadline is neither0 nor
UINT_MAX and raw unsigned now<deadline. Otherwise it leaves outputs unchanged:
inventory callers initialize0/0, while ordinary accessors retain their prior
combined result. Native writes include393a95f8's UINT_MAX and393a8b04/0b's
local-now+2000 or0; this audit does not infer a replacement producer from casts
or received packets.

`item_accessor_cooldown_oracle.py` /
`native-item-accessor-cooldown-oracle-20261018.json` execute **32 actual groups**
through both query paths and overrides, including failed base queries, absent
instances/descriptors, elapsed -1/0/1/599/600/601, deadline0/MAX/equality/active/
unsigned-wrap controls. Catalog/instance/combined-query and clock interfaces are
explicit inputs. Actual XlDiffTime executes against host MSVCR100 difftime64;
native x87 subtraction/truncation and u32 multiplication execute unchanged.
The std numeric_limits endpoint uses host MSVCP100. Source is archived in
`native-item-accessor-cooldown-source-20261018.txt`. Instance+1d/+1e receive
serializer/semantic mapping and genuine placement-lifetime input remain open;
no guessed summoned/last-despawn field or automatic selector activation is added.

### Native NumberOnly labels and raw text model (2026-10-18)

Label registration3924321c binds SetNumberOnly through typed thunk39243060,
parser39242f30 and binding39242e00 to setter3929eea0. The setter only writes
label byte+ad9; the constructor clears it. Actual Cry conversion3550a460 /
3550bf90 accepts Lua booleans or numeric nonzero (including NaN), rejects
strings/tables/nil/missing arguments, and returns no results. Changing the
flag does not reformat or clear existing text. Other widget classes do not
gain this Label-only method.

Label SetText3929f180 formats future input by grouping the last three BYTES
with literal commas, irrespective of digits, signs, existing punctuation or
Unicode character boundaries. Thus `1234` becomes `1,234`, `-123` becomes `-,123`
and `abcdefg` becomes `a,bcd,efg`. This path does not read the rich-text comma
option. The shared Lua SetText wrapper39267c10 /392674c0 /392655c0 requires
an actual string; invalid types leave a Label unchanged and return zero
results. The toolkit applies that source contract to Labels with either flag
state, preserving other widget classes' existing text behavior.

TextModel3922b680 compares the input C-string first, counts the structurally
valid UTF8 prefix through XlStringLen3301b000, rejects counts>=512, clears
both buffers when the prefix count is0, otherwise copies up to2048 raw bytes
and converts to UTF32 through3301a100. Grouping can split multibyte input:
three e-acute characters retain malformed grouped GetText bytes while drawing
only the first e-acute; two e-acute characters clear the model. GetText39267350
through Label3929ee40 returns one string containing the raw stored bytes,
independently of the rendered prefix. LabelData now preserves that distinction;
fonts, auto-sizing and fitting consume the decoded text.

The native decoder skips one leading BOM while retaining the original prefix
count as its output limit, and accepts structurally valid overlong encodings.
The toolkit follows this for representable scalars. Native UTF32 can also
hold surrogates or values above Unicode, and malformed BOM-tail input can
read beyond its C-string terminator. Those inputs remain an explicit backend
boundary: the toolkit refuses the change and retains the old raw/rendered
model instead of inventing replacement glyphs or reading past the terminator.

`label_number_oracle.py` /
`native-label-number-oracle-20261018.json` execute **104 actual-native groups**
through both typed wrappers, the flag setter, byte formatter, native text
model/XL conversion and GetText. Allocation, Lua stack primitives and memory
interfaces are explicit endpoints; formatting/counting/decoding run unchanged.
Cases cover default/repeated toggles, argument kinds and arity, failed Label
cast, NUL, grouping/punctuation/non-ASCII,511/512 count and384/385 grouped-digit
boundaries, raw-buffer truncation, BOM and malformed sequences.
`native-label-number-source-20261018.txt` records the wrapper/class/model and
XL/Cry source. Original money, item/cooldown stack, proficiency point and
character stat labels call this API; their scripts/assets remain unchanged.

Fresh crate check and example build pass. **55 selected tests pass** across
NumberOnly6, auto-clip6, scaled fitting8, drawable text3, rich text9, HUD6,
constant cooldown7, captions6 and scale/clip4. New controls include the original
CreateTwoLineMoneyWindow/Update and CreateItemIconButton/SetStack constructors,
strict physical item-button picking/clicking, typed wrappers, raw/display
distinction and native storage boundaries.

Final aa-dev captures were inspected:
`shots/aa-ui/native-number-labels-final-720-20261018.png`
(1280x720/default0.85, **597.6-637.3 FPS**) and
`shots/aa-ui/native-number-labels-final-1080-20261018.png`
(1920x1080/1.0, **473.1-602.4 FPS**). They show native currency123,456,789 /12 /34,
item stack12,345 and plain/grouped label controls. The original stack's shadow
and lower-right alignment were checked against `reference/ingame/bag/real.png`;
that reference does not contain a large grouped count. A light diagnostic
backing makes the original black currency text readable. Initial dark-backing
diagnostics are not the final visual evidence. Both final runs report zero
script errors/WARN/ERROR and self-close (PIDs46068/13800); the copied executable
was removed. No app API, cooldown producer or native selector activation changes.

### Widget visible alpha and the independent Show fade factor (2026-10-18)

Native SetAlpha39267d00/39267620/392658c0 writes visible alpha+d8 without
clearing the alpha channel at+280..298. Its typed float argument requires a
number; nil, strings, booleans, missing arguments and failed widget casts
silently return zero results without changing it. It upper-clamps values>1;
negative values and unordered NaN survive. A direct visible-alpha override
lasts until the next active channel step. The toolkit now preserves that
internal/current versus visible distinction instead of canceling the channel.

Show39266130 validates its first argument as bool or numeric nonzero, optional
duration as an actual number (invalid/missing means0), and returns zero values.
Its optional third float does not replace the manager's0/1 target. Helper
3927e420 compares the widget's own shown+ba first: a repeated value leaves
the existing fade record and callbacks alone. A transition writes shown
immediately and queues OnShow/OnHide, preserving both animation channels and
base alpha. This includes zero-duration transitions: completion occurs during
the manager frame, rather than resetting alpha synchronously.

Manager392aab40 uses separate fade factor+dc and pending+e0. A new show/hide
record starts at0/1 and targets1/0; replacement starts from the current factor
and retains its manager position. Start/end are absolute f32 milliseconds.
The UI clock39ac67f0 accumulates each f32 frame delta at3921c026..3a; the
toolkit keeps a dedicated f32 clock without changing its other f64 clocks.
Frame392ab0e0 completes end<=start (including unordered end/start), otherwise
evaluates `(now-start)/(end-start)` without a lower clamp. Completion writes
the factor, clears pending and dispatches OnEndFadeIn/OnEndFadeOut. Negative,
NaN and infinite durations retain the executed native comparison behavior.
Pending records request painting even when the factor is unchanged, so native
admitted draw clocks do not stall at infinite duration or rounded f32 time.

Native records are visited in registration order, with completion delivered
before evaluating the next record. A callback can replace a later record or
append one that is visited in the same frame. The toolkit retains an explicit
sequence on each active record; replacement keeps it and new records append.
Frame3921c0f9→392ab5f0 delivers manager completions before
3921c13c→392a9e40→3927de60 steps widget animation channels, so a completion
callback observes pre-step alpha and can start an admitted channel for that
same frame. Generic hidden-widget animation-clock policy remains outside this
slice; the source-order regression uses an admitted shown widget.

Paint3927b010 admits shown OR pending-fade widgets, while hit3927c6d0 requires
the shown flag. The renderer therefore has a distinct fading paint traversal;
input/public ancestor visibility remain strict. Each widget forms f32
`factor*base` before multiplying the parent's inherited alpha in parent-first
order (3921a870). Existing ancestor clipping and layers still apply. A fading
hidden ancestor carries its visible descendants until completion, then excludes
the subtree. EnableHidingIsRemove waits for the actual fade completion and
keeps descendants valid through its callback before retiring input/event state.

Offline artifacts under `data/extract/aa-ui/` execute **92 actual-native
fade/visibility controls**: `widget_visibility_oracle.py`51 full-wrapper and
manager cases; `widget_fade_oracle.py`36 frame/draw/hit cases; and
`widget_fade_order_oracle.py`5 ordered-record cases. The latter executes actual
Show/manager replacement through an in-emulator callback trampoline; callback
retirement/list insertion, allocation, layer interfaces and final renderer
delivery are explicitly controlled endpoints. It covers replacement position,
observation/mutation of a later record, retirement and appending. Corresponding
`native-widget-*-oracle-20261018.json` files retain outputs, and
`native-widget-visibility-source-20261018.txt` archives the wrapper, manager,
clock, paint, hit and frame-order disassembly. No editor/global modal policy
or foreign runtime clock values were inferred.

Fresh crate/example checks and the final example build pass. **103 selected
tests pass** (one pre-existing optional font-metric test ignored): widget fade12,
animation10, popup9, attachment5, scale/clip4, unit22, chat alpha6/chat clip7,
tooltips6, native slot selector6/mask8/completion7/state1. Controls include
strict wrapper arity/casts, nonfinite/negative values, f32 clock precision,
active alpha/scale preservation, parent alpha/clipping/layers, physical hit
exclusion, reentrant callbacks, ordered mutation and descendant retirement.
Original popup/confirmation, tooltip, arrow/window animation, chat and ordinary
slot script checks pass. Existing cache tests explicitly settle the new native
zero-duration Show record with tick0 before measuring later changes; no
animation elapsed time or restart is hidden by that settlement.

Final aa-dev captures were inspected, both with explicit `--shot-after 8`:
`shots/aa-ui/native-widget-fade-active-final-720-20261018.png`
(1280x720/default0.85, **465.9-632.1 FPS**) and
`shots/aa-ui/native-widget-fade-complete-final-1080-20261018.png`
(1920x1080/1.0, **518.6-615.2 FPS**). They use unchanged original CreateWindow
art/OnShow animation and an original CreateEmptyWindow control for base-alpha
reversal. The first shows active channels and hidden fade painting; the second
shows completion with the hidden subtree absent and .4 base alpha preserved.
Both report zero script errors/WARN/ERROR and self-close (PIDs39604/41288).
The copied executable was removed. Early fixture runs used an unavailable
CreateEmptyWindow-only helper or the default5s shot before transition; those
diagnostics are not final visual evidence. InheritAnimationData implementation
follows separately over these now-correct alpha/fade prerequisites.

### Native InheritAnimationData and widget scale lifetime (2026-10-18)

InheritAnimationData is now implemented over its actual typed thunk39267bf0,
parser39267400, binding39267120 and helper3927dd30. The native table parameter
resolves its widget host through `__this`; invalid/missing/wrong-class inputs
return zero results without copying. The toolkit accepts its registered live
widget tables, including hidden sources; it does not manufacture native foreign
userdata. No source visibility, parent, root or self exclusion is inferred.

The helper copies alpha endpoints/current/velocity/initial velocity/acceleration
at+280..294 and flag+298. Only an active copied alpha channel writes current+288
to visible+d8; an inactive channel preserves the destination's current visible
alpha while still replacing its configuration. Thus a source's direct SetAlpha
override is not copied in place of its active internal current. Old destination
channels stop silently; the copy does not restart them or emit end events.

Scale copies the five fields+29c..2ac, configured pivot+2b0 and flag+2b4. An
active copy writes the source's own raw+64 exactly, then settles the destination
geometry/anchors and dispatches OnScale42, even for self or unchanged raw scale.
It does not copy an effective ancestor scale, divide by the destination's UI
baseline during copying, or overwrite destination root/ApplyUIScale policy.
An inactive copy retains the destination's own scale and displayed geometry.
No elapsed/duration clock is copied: these native channels retain their current
velocity and current value, not an elapsed-time evaluator.

Configured and displayed pivots are now distinct. A scale configuration change
alone does not reinterpret current geometry. Actual3927b2f0 retains the chosen
point of the current rectangle; local scale-origin compensation expresses that
in the toolkit without changing authored anchors or copying motion/damage
offsets. Native geometry controls execute full pivot/extent arithmetic with no
authored anchor targets; general native anchor-provider calibration is not
broadened by this change. Original ordinary SetScale uses CENTER4, and the same
local settlement keeps existing center behavior while retaining copied pivots.

The next active scale step3927e0c9 normalizes current raw scale by the destination
root/UI baseline before integrating and multiplying back. This is deliberately
later than the copy. An executed edge copies1.700007677078247 as bits3fd999da;
the next root/UI0.85 tick0 becomes3fd999db. Both exact raw snapshots and an
independent `[7,-3]` damage/motion offset are checked through the Lua API and
the native-scale consumer. Existing general kinematic coefficient precision
remains separate; this slice copies the existing channel state rather than
replacing all animation timing formulas.

Full SetStart3927db80 controls also close a prior scale-stop error. Alpha stop
snaps an active channel to its target. Scale stop clears its flag at3927dbce
before the later scale branch, leaving own scale/geometry in place without
OnScale or completion events. Starting scale still settles geometry and emits
OnScale, including equal-scale starts. Ordinary constructor3927ac1e sets base
alpha/own scale1 before initializing endpoints1, coefficients0, flagsfalse and
pivotCENTER. A source with no animation state therefore retains that default
configuration even after a later visible-alpha override. The native prototype
constructor instead uses its prototype's current visible/raw values and resets
active flags; no prototype widget API was added.

Read-only offline artifacts under `data/extract/aa-ui/` execute **90 native
inheritance/start/update groups**: `widget_inherit_oracle.py`12 field/flag/self
cases, `widget_inherit_wrapper_oracle.py`11 full-wrapper cases,
`widget_inherit_geometry_oracle.py`13 full geometry cases and
`widget_start_inherit_oracle.py`54 constructor/start/copy/full-WidgetUpdate cases.
The matching `native-widget-*-oracle-20261018.json` files retain values and
`native-widget-inherit-source-20261018.txt` archives disassembly. Lua primitives,
class/script callbacks, root identity and profiling are explicit interface
endpoints; channel copy/flags, scale normalization and geometry arithmetic run
unchanged. Root-parent, unequal scale, active/inactive and self cases are covered.

Crate check and final example build pass. **120 selected checks pass**, with
one pre-existing optional font-metric test ignored: inheritance8, units23,
animation10, fade12, scale/clip4, attachment5, popup9, scaled fitting8, chat
alpha6/clip7, tooltips6, native selector6/mask8/completion7/state1. Original
BuffUpdate first starts ID11, starts ID22 forty milliseconds later, then removes
ID11 after another twenty milliseconds. The newer second button's different
alpha/scale is copied into the first button and continues instead of restarting;
the actual transformed icon remains the physical hit target. Other checks cover
strict arguments/casts/arity, inactive visible/config state, self callbacks,
raw-scale bits, constructor defaults, changed configured pivots and scale stop.

Final aa-dev shots with explicit `--shot-after 8` were inspected:
`shots/aa-ui/native-buff-inherit-active-final-720-20261018.png`
(1280x720/default0.85, **281.0-563.2 FPS**) and
`shots/aa-ui/native-buff-inherit-complete-final-1080-20261018.png`
(1920x1080/1.0, **501.4-618.0 FPS**). They use unchanged original buff-window,
item-icon and remaining-time scripts/assets, with a diagnostic parent scale3
to make the short animation visible. Both report zero script errors/WARN/ERROR
and self-close (PIDs36492/20432); the copied executable was removed. Initial
diagnostics overlapped the fixture status label at active scale; the final
board provides space without changing original widget extents or animation.

### Native running widget-channel configuration (2026-10-18)

SetAlphaAnimation and SetScaleAnimation now follow their complete typed
thunks39267dd0/39268bc0, parsers3922f260/392679e0 and bindings39265490/392654f0.
All four scalar arguments must be actual Lua numbers, narrowed to f32; numeric
strings, bools, tables and missing values silently return zero results without
partially changing configuration. Scale additionally requires a fifth string.
Extra arguments are ignored and a borrowed method on a drawable fails the
widget cast. Original pivot lookup3922c0a0 uses case-sensitive XlCompareString
and a C-string prefix: CENTER is recognized, center is unknown, and a NUL after
CENTER terminates the name. Unknown enum9 has the same zero-origin geometry as
TOPLEFT in392772b0; the toolkit retains that geometry equivalence rather than
silently selecting CENTER. The default unconfigured pivot remains CENTER4.

Actual configurators392771c0/39277230 write endpoints, current velocity and
initial velocity immediately, even while active. They preserve current visible
alpha/raw scale, active flags and displayed geometry. With f32 delta=to-from,
velocity=delta/velocityTime; nonzero accelerationTime writes
`2*(delta-velocity*accelerationTime)/(accelerationTime*accelerationTime)` in
native operation order. Exactly +/-0 retains the previous acceleration.
Negative values and NaN are not sanitized: NaN accelerationTime writes NaN;
zero velocityTime can write infinity. Restart restores the saved initial
velocity and initial value, retaining the configured acceleration.

Only these widget channels now use native SSE-f32 integration. Full
WidgetUpdate3927de60 computes dt=frameMilliseconds*.001f32, updates velocity
before current, then compares current against the endpoints to finish. No
analytic duration shortcut forces completion for zero/negative times or
unordered values. The shared effect/motion Timing evaluator is unchanged.
An actual sequence starts .2->1 with times .5/.4, steps80ms, then configures
.1->.9 with times .3/0 without restarting. Current stays3eae7d57 at configure
and tick0, then becomes3eca34b4 after20ms and3ef4b3ae after30ms. Both alpha and
scale retain those exact bits, and restart resets current to3dcccccd before
the next step. Nonzero replacement and negative-time controls are also exact.

Raw nonfinite channel/geometry state remains observable. Native Image39220590
does not reject such coordinates; its actual transform/+0.5/CVTT path collapses
them to integer-indefinite -2147483648 vertices. Bevy uses floating position
buffers, so its mesh-upload loop alone omits quads with nonfinite rectangle or
free-corner coordinates. This is an explicit app rendering boundary, not a
claimed native argument rejection, clamp, paint-admission change or timer stop.
A CPU upload integration executes real raw-zero-time scale, retains NaN/current
active state, uploads the finite neighboring quad, and verifies actual mesh
positions while rejecting a nonfinite free-corner route as well.

Read-only offline artifacts in data/extract/aa-ui execute **211 native groups**:
widget_channel_oracle.py86 configure/start/full-update/pivot cases,
widget_channel_wrapper_oracle.py120 full Lua/Cry conversion cases, and
widget_channel_draw_oracle.py5 actual image-consumer cases. Matching
native-widget-channel-*-oracle-20261018.json and
native-widget-channel-source-20261018.txt retain results/source. Lua stack
primitives, widget/class callbacks, root identity and final texture submission
are explicit interface endpoints; native configuration/integration, pivot
lookup, geometry and image conversion arithmetic execute unchanged.

This slice does not change channel callback ordering. The native scale-end
callback precedes its final raw-scale/geometry write, whereas toolkit end
notifications are currently delivered after its channel pass; callback-driven
reconfiguration at that boundary remains a separate source/lifetime task.
The existing public frame-time sanitation/hidden-channel scheduling and general
anchor calibration are also unchanged.

Crate check, example build and **126 selected checks pass**, with one existing
optional font-metric test ignored. These cover five new strict/configuration/
native-bit/physical-window tests, 24 units including actual mesh upload,
inheritance8, animation10, fade12, scale/clip4, popup9, attachment5, scaled
fitting8, chat alpha6/clip7, tooltip6, slot selector6/mask8/completion7/state1.
The original CreateWindow/SetttingUIAnimation/OnShow scripts and original button
skin run unchanged in native_widget_channel; the fixture changes configuration
during active opening and checks transformed physical button targets.

Final aa-dev captures with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-widget-channel-active-720-20261018.png
(1280x720/default0.85, **361.8-580.2 FPS**) and
shots/aa-ui/native-widget-channel-complete-1080-20261018.png
(1920x1080/1.0, **485.1-529.9 FPS**). The first shows retained-current state and
different retained/replaced coefficients; the second shows final window art
and buttons without overlap. Both report zero script errors/WARN/ERROR and
self-close (PIDs44668/30676); the copied executable was removed. No original
Lua script, font calibration, root API or ordinary-slot policy was changed.

### Native widget-channel end callback continuations (2026-10-18)

The per-widget channel pass now releases Ui borrows and delivers end handlers
at their actual native positions. Alpha completion first writes its target and
clears its active flag, calls OnAlphaAnimeEnd, then reads the scale flag. A
scale enabled by that callback therefore receives the current frame's step.
Restarted alpha is not stepped twice, and alpha enabled by scale-end waits for
the next frame because its native phase has already passed.

Scale completion3927e149 clears the flag while retaining old own-scale/displayed
geometry through OnScaleAnimeEnd. Its old finishing target is captured before
the callback. After the callback the native continuation writes that captured
value with the live pivot/UI policy, settles geometry/fitting, and dispatches
OnScale. New configuration, initial/current velocity and active flags survive;
the backend does not restore a cached whole state over callback changes.
Thus a native end callback sees old1.01/flagfalse, starts .5->1.25 and gets a
nested OnScale(.5), then its old frame still emits OnScale(1.0) with the new
flagtrue. The next step starts from raw1.0 with the new velocity. Likewise an
alpha-end callback starting scale1.5->1 in.1s sees nested1.5 followed by the
same frame's1.4 after20ms. Hidden live widgets finish their continuation before
deferred retirement; application-retired subtrees are not dereferenced.

Actual class callbacks3927bba0/3927bbb0 dispatch event39/40 through3927b8b0.
They require a registered handler, an enabled script VM and a successful
BeginCall. Argument helper39279ae0 retains/releases and pushes only the widget
self table, with no extra end-handler arguments. Native event42 is OnScale.
No extra constructor/type/time admission gate is inferred. Existing native
method wrappers already call flush immediately after their closures, so their
queued SetScale/SetStart/Inherit OnScale handlers already execute before the
method returns to Lua, including inside another handler. Those wrappers were
preserved; the initial idea of replacing their callback delivery was withdrawn
after inspecting that complete existing call chain.

widget_channel_callback_oracle.py executes **15 native callback groups**,
including full WidgetUpdate, real class helpers, real admission/self arguments,
actual configure/start/geometry and reentrant machine-code continuations. Only
script VM interfaces are controlled endpoints; the trampoline's translation
cache is invalidated when its code changes. Artifacts
native-widget-channel-callback-oracle-20261018.json and
native-widget-channel-callback-source-20261018.txt retain observations, the
actual class vtable/event mapping and source. A separate four-group
widget_channel_frame_oracle.py executes event9/10 and shown-child recursion for
future frame-policy work. It is not evidence that this slice changes global
OnUpdate/OnEffect timing, active-ID traversal or ancestor visibility clocks.

The new original Achievement fixture retains its actual constructor, OnShow,
OnScaleAnimeEnd, OnScale and category AdjustScale functions. A diagnostic
observer delegates to the original AdjustScale and records old scale during
the end handler, then final scale from the handler it installs. The original
close button remains the strict transformed physical hit target and closes the
window. No original handler or fixture extent was replaced to hide an ordering
failure. Other temporal controls cover nested restart/captured target, same-
frame cross-channel enable, no second alpha step, synchronous method return,
handler release, hiding and deferred retirement.

Crate check/example build and **132 selected checks pass**, with the existing
optional font-metric test ignored: channel11, units24, inheritance8, animation10,
fade12, scale/clip4, popup9, attachment5, scaled fitting8, chat alpha6/clip7,
tooltips6 and slot selector6/mask8/completion7/state1. Both final aa-dev captures
with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-widget-callback-achievement-720-20261018.png
(1280x720/default0.85, **420.8-564.4 FPS**) and
shots/aa-ui/native-widget-callback-achievement-1080-20261018.png
(1920x1080/1.0, **517.0-597.6 FPS**). Each shows the original Achievement view
with the observer reporting end0.990000 and final OnScale1.000000. Both report
zero script errors/WARN/ERROR and self-close (PIDs5492/19444). The copied
executable was removed; no root API, Lua asset, font or global frame policy was
changed.

### Native widget frame admission and handler/channel order (2026-10-18)

The widget pass now follows manager392a9e40 and WidgetUpdate3927de60: for
each admitted widget, call OnUpdate(self,dtMs), then resolve/call the live
OnEffect handler, step that widget's alpha/scale channels with their immediate
end continuations, and visit its live shown-child vector in order. An OnUpdate
that starts a channel therefore advances it in the same frame. This corrects
the prior globally batched handlers-after-channels pass and its alphabetic
OnEffect-before-OnUpdate ordering. Handler replacement/release is observed at
the actual invocation position.

Retiring a subtree now clears its widget channel/current-scale/offset caches
through the existing Animations::remove path, including paused configured
channels. Those caches must not rely on a later all-active scan: the native
admission traversal correctly never visits destroyed objects.

Manager392a9e40 walks all eight native layers from bottom to top. Before each
layer it copies that layer's current root list through392a9b70/392a93c0.
Showing a root in the current layer during a callback defers its visit until
the next frame; showing it in a later layer admits it this frame. A root moved
to a later layer can run twice, as the native copied lists do. An already
admitted root does not recheck its own shown flag after another callback hides
it. Root Show/hide-fade manager membership is represented by shown-or-fading;
actual hide completion392ab2be..339 removes the native root before the widget
pass. Shown children of a hidden but still-admitted root continue updating.

Children use a different gate:3927e1f2 checks only each child's own shown byte.
A hidden fading child is excluded. The end pointer is read live, allowing a
parent or earlier child to append another shown child for this frame. Alpha0,
offscreen coordinates and drawing clips do not suppress this update clock.
Application-retired objects are safely excluded before further Lua/tree access;
this does not emulate dereferencing a destroyed native widget.

widget_frame_manager_oracle.py executes **25 native manager/frame groups**,
including real eight-layer copying, WidgetUpdate, script admission, actual
Show3927e420/manager392aab40 and hide completion392ab0e0. Allocation, class/VM
interfaces, profiling and manager identity are explicit endpoints; ordering,
shown checks, channel arithmetic and manager membership changes execute native
instructions. Dynamic child-vector changes are identified synthetic callback
side effects. Artifacts native-widget-frame-manager-oracle-20261018.json and
native-widget-frame-manager-source-20261018.txt preserve results/source.

The original battlefield waiting_alarm.alb provides the concrete regression:
its real alpha-end callback installs a ten-second OnUpdate hold; reaching that
hold starts the original .5s fade. The same20ms frame now reaches alpha.96
instead of leaving1 until the next frame. Its original subsequent alpha-end
requests automatic results and hides the alarm. The fixture supplies a small
received-data table and records that external scoreboard endpoint while keeping
the constructor, artwork, configuration and all alarm handlers unchanged.
The fixture observes/delegates registration while loading this one original
asset to retain its actual INSTANT_GAME_END callback; the factory/helper are
local upvalues, not globals supplied by a new toolkit API. It also checks that
the handler installed by alpha-end does not consume that already-passed500ms
frame: after the next9980ms the hold still has20ms remaining.

Other per-class passes (chat/slot/statusbar etc.) and drawable timelines retain
their existing scheduling. This slice does not claim to move every native
class virtual+158 update into its exact per-widget position, nor change public
frame-time sanitation, layout calibration or application cooldown producers.

Crate check/example build and **193 selected checks pass**, with one existing
optional font-metric test ignored: units24, frame10, channels11, animation10,
inheritance8, fade12, attachment5, scaled fitting8, scale/clip4, chat alpha6/
clip7/lifecycle9/lock7/method5/reset7, drag12, keyboard4, resizing4, popup9,
HUD6, tooltip3 and slot completion7/mask8/selector6/state1. The tightened
original hold-lifetime assertion also passed separately. This includes actual
original Achievement, staggered buff reorder, physical chat controls, popup
retirement, transformed window buttons and original tooltip-scale regressions.

Both aa-dev captures with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-widget-frame-alarm-720-20261018.png
(1280x720/default0.85, **349.8-468.3 FPS**) and
shots/aa-ui/native-widget-frame-alarm-1080-20261018.png
(1920x1080/1.0, **347.3-392.1 FPS**). The original Arena Ended artwork is in
its active fade and the observer records same-frame alpha0.960000, without
overlapping the report. Both log zero script errors/WARN/ERROR and self-close
(PIDs28044/2104); the copied executable was removed. No original Lua asset,
font calibration, root integration API or ordinary-slot policy was changed.

### Native button state colors and original empty-stack icons (2026-10-18)

SetNormalColor, SetHighlightColor and SetPushedColor now recolor the button's
existing background for that state, followed by its existing matching state
drawables. Hidden matching artwork is included. They do not change child text,
unrelated artwork, disabled artwork, input enablement or widget alpha, and they
do not remember a palette for images created or bound afterward. Shared image
aliases retain their shared mutation behavior. The methods remain limited to
their native button/checkbutton/cooldown-button surfaces; ordinary slots,
labels and drawables do not acquire them.

Actual four-number thunk3922f790/parser3922f260 and the Cry script argument
conversion execute in button_state_color_oracle.py. Normal3922cdc0,
highlight3922d060 and pushed3922cf10 cast the receiver through interface+14,
then call392806c0 for the current background and39282f30 for the matching
state list. All four arguments must be actual numbers; invalid input or a
failed button cast silently returns no values, and extra arguments are ignored.
Raw negative/out-of-range/nonfinite values narrow to f32 before the native
Color copy3922b330. The **33 executed native groups** retain target order,
payloads, no-art/no-future-palette behavior and strict argument gates in
native-button-state-color-oracle-20261018.json and
native-button-state-color-source-20261018.txt. Script stack/return, cast and
virtual drawable SetColor endpoints are explicit controls. This is not a full
native raster/color-quantization oracle.

The concrete original consumer is cooldown_action_button.alb's
ProcessEmptyStackColor: true applies white alpha0.3 to normal, pushed and
highlight images; false restores white alpha1. Its original SetItemIcon,
SetItemIcons and SetButtonBackground build the four item-icon state images;
the independent disabled image retains alpha0.5. The fixture executes that
bytecode unchanged with item4045/grade2. It does not infer the empty flag from
SetStack, which only updates the stack text. Ordinary slot can_use=false black
overlay policy remains separate.

Targeted tests check strict arguments/return arity, raw f32 payloads, current
hidden artwork, aliases, late-created images, original icon quads and physical
hover/press/release. The dimmed original button stays clickable and executes
its original OnClick before the application OnClickProc endpoint. Disabling
and then restoring it selects the unchanged disabled artwork and restored
normal artwork respectively. Crate check/example build and **75 selected
checks pass**: state colors5, drawable color2, frame10, drag12, keyboard4,
attachment5, popup9, HUD6 and slot completion7/mask8/selector6/state1. The five
new checks also pass separately after the final fixture-caption refinement.

Both final aa-dev captures with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-button-colors-normal-final-720-20261018.png
(1280x720/default0.85, **315.6-462.2 FPS**) and
shots/aa-ui/native-button-colors-pressed-final-1080-20261018.png
(1920x1080/default1.0, **332.2-425.5 FPS**). The second fixture positions and
presses the virtual physical pointer on the dimmed original button; CPU tests
also verify its strict hit, matching pressed icon quads and release callback.
Both report zero script errors/WARN/ERROR and self-close (PIDs1648/48024).
The copied executable was removed. No app hook, Lua asset, font, shader or
cooldown timing policy changed.

### Native checkbutton artwork visibility and replacement lifetime (2026-10-18)

Checkbuttons now write the actual shown flags of their bound checkmark artwork,
instead of applying a separate checked-state predicate while drawing. Native
39281440/392814c0 write the normal mark first and disabled mark second. Enabled
checked controls show the normal mark; disabled checked controls show only the
disabled mark, with no normal-mark fallback when that binding is missing.
Unchecked controls hide both. Aliased marks retain that write order.

SetChecked repeats reassert those flags even when the checked value is unchanged;
they still emit OnCheckChanged only on a changed value with its optional event
flag enabled. Explicit drawable SetVisible lasts until the next applicable
state write. Rebinding writes only the currently bound marks: an old image
retains its previous shown flag. Previously, replacing an unchecked bound mark
could expose that hidden image as unrelated artwork.

The actual typed SetChecked thunk39230c50/parser39230ae0 and binding39230770
require a boolean or native numeric boolean for the first argument. Missing,
nil, strings, tables and wrong receivers silently return no values. The second
argument defaults to true when absent or of an invalid type. The checked-art
thunk39230c70/parser39230b90/39067ad0 and bindings392307e0/39230960 require a
drawable object table; nil is not an unbind request. They return no values.
Retired toolkit objects remain a safe application refusal boundary.

Changed Enable calls reach checkbutton virtual+148 at392842a0 after the own
enabled byte is written. Repeated Enable with the same value does not reassert
art. Native3927b290 processes children in order before the parent when recursive
enable is requested; the toolkit now keeps that ordering, including shared mark
images. Generic Enable argument coercion and the broader native button-state
continuations are outside this slice.

checkbutton_oracle.py executes **91 native wrapper/visibility groups**, including
actual Cry argument conversion, both bindings, visibility writes, alias/rebind
lifetimes, native Enable3927b290 and its real checkbutton vtable continuation.
Lua stack/table/reference/cast and script callback delivery are explicit
endpoints; the final ordinary button-state continuation is recorded separately.
Artifacts native-checkbutton-oracle-20261018.json and
native-checkbutton-source-20261018.txt preserve observations and source. This
does not claim native rendering of arbitrary aliased ordinary-state images or
dereferencing a destroyed native drawable.

Six new checks cover strict inputs, no disabled fallback, manual visibility,
repeat/rebind lifetime, aliased marks, recursive ordering and callback
reentrancy. The actual original CreateCheckButton, SetEnableCheckButton and
OnCheckChanged bytecode remain unchanged. Strict physical clicks on both its
checkbox and its text run the original handlers and application callback;
the original Achievement row-checkbox regression also passes. Crate check,
example build and **101 selected checks pass**: checkbutton6, natives21,
event handlers5, drawable color2, button state colors5, chat context3,
list geometry4, popup9, drag12, animation10, fade12, attachment5, active tabs6
and ordinary slot state1.

Both aa-dev captures with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-checkbutton-lifetime-720-20261018.png
(1280x720/default0.85, **363.8-416.7 FPS**) and
shots/aa-ui/native-checkbutton-lifetime-1080-20261018.png
(1920x1080/default1.0, **320.0-417.7 FPS**). They show the original checked,
unchecked and disabled checked artwork, an explicitly hidden mark, and a mark
restored by a repeated SetChecked without changing its value. Both report
zero script errors/WARN/ERROR and self-close (PIDs4252/16700); the copied
executable was removed. No original Lua asset, app API, font or timer changed.

### Native ordinary button artwork and pointer-state lifetime (2026-10-18)

Ordinary button state is now retained independently of the renderer's pointer
snapshot. Native39282f70 hides the currently bound backgrounds and state extras
in Disabled/Normal/Highlighted/Pushed order, then shows only the selected
state's artwork. A missing state has no normal-background fallback. The toolkit
uses those actual shown flags, so explicit drawable SetVisible survives drawing
until another state write. Old background replacement392840d0 explicitly hides
the previous image before rebinding and reapplying the current state. This
differs from the checkmark replacement lifetime described above.

Aliased backgrounds preserve ordered writes; late-created state extras reapply
the current state, including the native creation-failure continuation. Actual
CreateStateDrawable39284b80/list insertion39282930 and state application execute
in button_art_creation_oracle.py. Drawable construction and heap allocation are
controlled endpoints. Existing supported toolkit drawable factories are retained;
this does not close every unsupported factory type or its typed Lua wrapper.

SetButtonState's typed thunk3922f5e0/parser3922f100/binding3922d3c0 requires an
actual string. NUL-terminated, case-insensitive DISABLED/HIGHLIGHTED/PUSHED names
select those states; other strings select NORMAL. A disabled widget forces
DISABLED. GetButtonState3922e510 returns the exact uppercase name. Background
thunk3922f5c0/parser3922f040 and bindings3922d670/d7f0/d970/daf0 require a live
drawable object table: nil is not an unbind request. Invalid arguments or failed
casts silently return no values. Retired objects are safely refused by the app.

Native enter39284090/leave392840b0 write state before their Lua handler. A changed
Enable continuation39283ff0 disables the art immediately; reenabling a previously
disabled state uses the current hover state. Repeated Enable does not rewrite
art. Pointer down39283a80 selects PUSHED before OnMouseDown; pointer up39283c30
delivers OnMouseUp while pushed, then rereads current state/registration before
restoring HIGHLIGHTED. Handler changes and rebindings therefore survive the
continuation. RegisterForClicks3922e490/392812c0 uses a case-sensitive actual
string and an optional literal boolean, otherwise true. Fresh initialization
3928397e registers LeftButton only; final39283a44 selects NORMAL.

button_art_oracle.py, button_pointer_oracle.py and button_art_creation_oracle.py
execute **227 native groups** (86 artwork/bindings, 105 pointer/state/registration/
getter and 36 creation/list/failure cases). Their native-button-art*,
native-button-pointer* JSON and native-button-art-source-20261018.txt artifacts
preserve source, observations and controlled Lua/cast/allocator/script-event
endpoints. The existing click/capture/base-widget fallback policy is unchanged;
registration now governs the source-proven pressed-art transitions. Nondefault
native block-during-fade admission remains open. Drawable image/icon
SetMask/SetMaskCoords/SetMaskVisible and hover-mask rendering remain a separate
backend gap; no mask, GCD, selector or timer policy is inferred here.

Nine new checks cover strict wrappers, no fallback, manual visibility, aliases,
old/late artwork, enable/repeat behavior, callback rebinding and physical pointer
order. The unchanged original CreateEmptyButton/ApplyButtonSkin(DEFAULT),
SetBGPushed/SetBGHighlighted and dimmed CreateCoolDownActionButton consumers run
with their original physical handlers. The existing state-color fixture now
uses a separate bare button for future-art assertions instead of assuming nil
unbinds. The selected-mask geometry fixture uses SetButtonState explicitly
instead of mutating the hover snapshot of a bare handler-free cooldownbutton;
its quads must remain nonempty and retain their original geometry assertions.

Crate check/example build and **168 selected checks pass**: new art9, colors5,
generic mask10, plus144 broader checks covering checkbuttons, pet/event listeners,
original action/equipment slots, unusable-skill overlay, achievement rows, active
tabs, chat context, popups, drag, attachments and widget animation/fade/frame order.
The final focused24 checks also pass after the fixture correction.

Both aa-dev captures with explicit `--shot-after 8` were inspected:
shots/aa-ui/native-button-art-normal-720-20261018.png
(1280x720/default0.85, **328.9-400.5 FPS**) and
shots/aa-ui/native-button-art-pressed-1080-20261018.png
(1920x1080/default1.0, **334.4-407.4 FPS**). They show original normal/highlighted/
pushed/disabled skins, retained selection aliases, explicitly hidden artwork and
the dimmed item icon. The second uses an actual pointer press. Both report zero
script errors/WARN/ERROR and self-close (PIDs46756/24964); the copied executable
was removed. No original Lua asset, app API, font or animation timer changed.

### Native drawable masks and original item hover feedback (2026-10-18)

Image and icon-image SetMask/SetMaskCoords/SetMaskVisible now retain a real mask
texture, independent normalized UV rectangle and independent visibility byte.
Fresh image39290863/icon3928fe98 initializes that byte false. Loading a mask
does not change it; successful reload resets the UV rectangle to the whole
texture. Failed loading retains the previous texture, coordinates and flag.
There is no nil unbind or fallback image. Native texture ownership methods
392892b0/39289770 release the previous reference before acquiring the new one.

Actual image bindings39234090/39234100/39234190 and icon bindings
39234310/39234380/39234410 cast through their distinct image interfaces.
SetMask requires an actual string, SetMaskCoords four actual numbers and
SetMaskVisible an actual boolean or numeric boolean. Missing/invalid values and
failed casts silently return no values. Numeric boolean comparison keeps its
double precision, including tiny/subnormal nonzero values. Native coordinate
methods392891c0/39289680 narrow inputs to f32, multiply by reciprocal texture
dimensions and use endpoint1 independently for a zero width/height. Negative,
fractional and nonfinite raw coordinates are retained.

The offline drawable_mask_oracle.py executes **86 wrapper/lifetime/draw controls**:
64 argument/ownership/invalid-order cases and22 actual image/icon submissions.
The real Cry argument converters, bindings, refcount/UV methods and draw
3928b420/3928ed90 through39220b90 execute; final3921fc30 submission captures all
six native vertices, image/mask UV streams and RGBA8. Lua stack/cast, resource
loading, texture dimensions/reference interfaces and final GPU submission are
explicit controlled endpoints. Resource-manager editor/path-prefix behavior is
not inferred from those loader controls. Native no-texture image coordinates
are inert; icon coordinates dispatch through a null texture interface. The app
refuses that invalid icon ordering safely. Retired objects are also refused.

drawable_mask_state_oracle.py executes another **64 native state controls** with
actual image/icon availability and visibility virtual methods. The combined
**150 groups** and source are retained in native-drawable-mask-oracle-20261018.json,
native-drawable-mask-state-oracle-20261018.json and
native-drawable-mask-source-20261018.txt. Button state application first disables
the former highlight background mask, then enables loaded masks of current
highlight backgrounds/extras during its hide pass, even if that art stays hidden.
Explicit mask visibility survives drawing until another applicable state write.

The original slot.lua SetDefaultMask consumer loads ui/common/slot_over.dds,
sets its40px rectangle, hides it and reapplies the highlight background. Hover
therefore uses the original mask with each icon layer. Icon3928ed90 shares its
primary normalized image UV with every retained layer; the previous whole-UV
overlay shortcut is corrected for both masked and unmasked icon drawing.
The existing image-plus-mask Material2d already matches the original x2ui.cfx
blend and samplers, so the shader is reused unchanged. Rotation, parent/UI scale
and scissor clipping carry unit UVs associated with both source rectangles.

Native3921f2a0 clamps source color, multiplies in x87 precision and truncates
to RGBA8. Masked images and cooldown fans share that conversion, including
NaN-to-byte0 and infinity saturation; authored/raw color storage is unchanged.
The rendered-hover state-color regression now checks alpha76/255 for authored
alpha0.3, while its unmasked normal/pushed assertions still check0.3. Native raw
nonfinite UVs remain in CPU state/quads, but the Bevy upload boundary refuses
their undefined GPU material sampling. A meaningful upload test verifies no
mask uniform/material is allocated and finite neighboring geometry still uploads.

Hover and cooldown masks have distinct source roles. Generic cooldown native
3928b3c0/3928ed10 draws base image geometry with its separately supplied timer
mask, ignoring the drawable's own hover mask. A shared geometry implementation
provides that base path without mutating the tree. This prevents a real hovered
action button from losing its cooldown sweep when its ordinary image becomes
masked. No timer, selector, GCD policy or application API changes.

Crate check/example build and **215 checks pass**, with one existing optional
font check ignored: focused59 (including nine new mask tests and two new upload/
RGBA8 library checks), plus156 original consumer/input/animation/clip regressions.
The original dimmed CreateCoolDownActionButton physically enters, presses,
releases and clicks; hover masks appear while its fan triangle count and
original OnClickProc callback remain intact. Additional checks cover strict
arguments, load/reload failure lifetime, stale objects, raw UVs, shared icon
layers, late highlights and rotated/scaled/scissor-clipped masked images.

Both aa-dev `--shot-after 8` captures were inspected:
shots/aa-ui/native-drawable-mask-hover-720-20261018.png
(1280x720/default0.85, **302.2-333.3 FPS**) and
shots/aa-ui/native-drawable-mask-hover-1080-20261018.png
(1920x1080/default1.0, **298.0-373.3 FPS**). They show original normal/cooling,
dimmed hovered/cooling and highlighted icons. Both report zero script errors/
WARN/ERROR and self-close (PIDs52348/34640); the copied executable was removed.

### Native button text palette and current style lifetime (2026-10-18)

The original ingameshop/confirm_view.alb:231 applies FONT_COLOR.DEFAULT directly
to details[i].textButton after CreateRadioButton has initialized its full button
palette. The unchanged radio_button.alb creates that text control through
CreateEmptyButton. Previously the renderer selected its palette again every
draw, discarding the shop's direct style color. It now paints the current text
style, with palette application at the actual native write points.

Native39280680 stores one of four raw RGBA entries and updates the current font
only when that entry equals the current button state. Native39282f70 reapplies
the selected palette after artwork changes, even for a repeated state. Direct
style:SetColor instead reaches3925b810/3925b1d0 and39274950, replacing only the
current font color. Drawing, unrelated palette entries and ordinary text/extent
changes preserve that override. A selected palette setter or a state/background
write restores the corresponding palette; this also applies while disabled.
The constructor loop392839a0..39283a3e initializes every entry white, without a
normal-state fallback. Native state indices0/1/2/3 mean disabled/normal/highlight/
pushed; the crate's existing palette order remains normal/highlight/pushed/disabled.

All four text palette setters and style:SetColor require four actual numbers,
narrow each to f32 and return no values. Missing, short, string/boolean/nil and
failed-class arguments are inert. Extra arguments are ignored. Raw signed zero,
negative values, NaN and infinity remain in current/palette storage. The existing
style:GetColor helper is retained; its native return shape is not claimed here.

button_text_color_oracle.py executes **47 actual native groups**, including the
real constructor initialization loop, strict four-number parsers and Cry
converters, all four bindings, palette/current-style storage, direct style
binding and state application. Lua stack/host casts/results, script callbacks
and unrelated drawable interfaces are controlled endpoints. No palette/color
selection or copy algorithm is replaced. Source and raw-bit outputs are under
native-button-text-color-source-20261018.txt and
native-button-text-color-oracle-20261018.json in the ignored aa-ui extract.

Crate check/example build and **178 checks pass**, with one existing optional
font check ignored: five new palette/current-style tests,147 checks across23
existing integration targets and26 library checks. Coverage includes the actual
unchanged shop view constructor, its original radio callbacks and strict physical
hover/press/release/choice/leave at1280/default0.85. The original direct color
differs from its normal palette, and unvisited rows retain it. Other checks keep
original button skins, disabled/checked art, masked/cooling item icons, chat tabs,
tooltips, list controls, popups, scale/animation/fade and retirement behavior.

Both aa-dev `--shot-after 8` captures were inspected:
shots/aa-ui/native-button-text-direct-720-20261018.png
(1280x720/default0.85, **346.2-411.4 FPS**) and
shots/aa-ui/native-button-text-pressed-1080-20261018.png
(1920x1080/default1.0, **364.7-461.6 FPS**). They use the original confirmation
view/radio constructors with four populated option labels; this is a control
fixture, not a purchase/network-flow implementation. The second uses an actual
pointer press. Both report zero script errors/WARN/ERROR and self-close
(PIDs45568/10092); the copied executable was removed. No icon/art color, timer,
selector, application hook or original Lua asset changed.

### Effect visibility and original low-health warning restart (2026-10-18)

The original hud/main_menu_bar/main.alb:42 and unitframe/player.alb:288 restart
their low-health effect only when its IsVisible() flag is false. A healthy
UpdateHpMp/UpdateAll stops the effect through SetStartEffect(false). The toolkit
already suppressed stopped effect art in its renderer, but retained shown=true;
that stale visibility blocked the next low-health warning. The actual unchanged
player UpdateAll regression failed on19def45 with "healthy menu border still
marked visible" before this correction. This was a restart-admission bug, not
an ordinary stopped-glow rendering bug.

Native SetStartEffect thunk39236830/parser39235f40, binding39234cc0 and setter
3928fb50 require an actual boolean or numeric boolean. Numeric comparison keeps
the original double precision, including tiny nonzero values and NaN. Invalid/
missing arguments or a failed effect-interface cast return no values without
changing state; extra arguments are ignored. The setter writes the drawable's
own shown byte for both start and stop. Stop preserves the phase/sample data;
start resets its color/motion initialization through3928df00/3928ea80. No new
widget callbacks, timer formula, effect phase selection or getter policy is
introduced in this slice.

effect_start_oracle.py executes **50 actual native groups** covering those full
wrappers and initializers, strict/failed-class gates, raw numeric booleans,
configured/empty color vectors, stop storage, manual visibility and restart.
Lua stack/host casts/results are controlled endpoints; raw phases are fixture
inputs, with no replacement visibility/reset algorithm. Raw outputs/source are
native-effect-start-oracle-20261018.json and
native-effect-start-source-20261018.txt under the ignored aa-ui extract.

The original LifeAlertEffect zero-HP branch calls SetVisible(true) separately.
An additional actual3928f810/3928e6c0 draw control proves that manual re-show can
advance color time while the stopped running flags remain false. The current
generic effect renderer's separate stopped flag and global clock did not yet
match that path. Native IsRunning also tests both color and motion flags;
its separate-channel getter/paint-clock contract was open in this correction
(implemented by the following milestone). This correction
does not claim those behaviors or change the existing ordinary-slot completion
clock and selector.

Crate/example check and example build pass. **72 selected checks pass** across
nine targets: four new effect start tests plus68 existing animation, ordinary
button/text, drawable-mask, fade/frame and slot completion/selector checks.
The genuine player UpdateAll recovery/low-health sequence now passes twice;
the unchanged action-slot autoattack/combo/toggle helpers stop and restart their
existing effect objects. Sample/configuration preservation and strict wrapper
arity/type/class gates are checked separately. No app-owned producer changed.

Both aa-dev `--shot-after 8` final captures were inspected:
shots/aa-ui/native-effect-restarted-low-final-720-20261018.png
(1280x720/default0.85, **242.3-290.2 FPS**) and
shots/aa-ui/native-effect-recovered-final-1080-20261018.png
(1920x1080/default1.0, **298.6-319.8 FPS**). They exercise the unchanged original
health warning and autoattack/combo/toggle consumers. The low-health border
returns after recovery; the healthy capture has no border. Both report zero
script errors/WARN/ERROR and self-close (PIDs26044/45844); the copied executable
was removed. The fixture invokes the real original UpdateAll without an
unnecessary ENTERED_WORLD event; an earlier diagnostic with that extra event
hit the existing fake battlefield maxCount=nil boundary and is not clean QA.

### Generic Effect admitted painting and independent native clocks (2026-10-18)

Original LifeAlertEffect's zero-HP branch manually SetVisible(true) after a
healthy SetStartEffect(false). Native3928f810 admits that effect by its shown
byte, irrespective of its stopped flags. The toolkit now advances generic
Effects at the actual owner paint traversal before the alpha/scissor skips.
Hidden owners/ancestors, hidden effects, missing membership and retired objects
pause; hidden fading owners, alpha zero, empty clips and offscreen rectangles
still admit the native clock. Update only requests painting; an unpainted
frame does not advance an Effect. Atlas frames and private ordinary-slot
edge/completion/cooldown clocks are unchanged.

Color3928e6c0 and motion3928f460 retain separate f32 seconds, delay/hold clocks,
phase indices, carry, repeat counters and running flags. Start initializes both
channels through3928df00/3928ea80. A finished phase hands off on a subsequent
draw and retains the new initial sample for that draw, carrying time without
skipping phases. Coefficients latch at initialization; live endpoints/timing
records do not retroactively replace them. Finite completion preserves the
last sample. Zero coefficients with different endpoints stay at the initial
sample. Raw internal nonfinite arithmetic is not normalized by the clock;
the existing finite-position GPU upload fence remains in force.

The constructor uses repeat0/counter1 for both channels and priority0/scalex
for a new color phase. The actual scaley branch3928e7e0 adds displacement to
final Y, unlike the other six priority branches; its native crossing behavior
is retained. Edge movement3928a950 multiplies before subtracting the half
extent as separate f32 operations. The existing correct Lua-facing radius
argument order and other effect transform geometry remain unchanged.

Registration39239704 names binding39235b00 **IsRunning**: it returns exactly
one boolean equal to color_flag AND motion_flag, with zero results on a failed
effect cast. The older toolkit IsStartEffect alias is retained as compatibility.
Native clear39290e20/39290e80 hides and stops both flags, clears only the chosen
phase vector, and preserves sampled values and the other channel configuration.
Manual visibility resumes admitted clock work without turning either flag on.
No widget callbacks or application producers are added. Existing configuration
setter coercion/index admission outside the source-closed ordinary inputs is
not revised or claimed by this clock slice.

The ignored aa-ui artifacts contain **58 actual native groups**:43 temporal
groups in effect_temporal_oracle.py,6 constructor/getter groups in
effect_clock_defaults_oracle.py and9 generic widget/layer/effect paint controls
in effect_paint_admission_oracle.py. The real initializer, kinematics, both
phase machines, interpolation, clear, cast/AND/result code and admitted draw
path execute; raw phase records and host/render/time endpoints are explicit
fixture inputs. No phase/sample/carry/repeat algorithm is mocked. Corresponding
native-effect-*-oracle-20261018.json outputs and
native-effect-clock-source-20261018.txt preserve the evidence.

Crate check/example build and **151 selected checks pass**, with the existing
optional font check ignored:12 new clock tests,14 animation/start checks,
26 library checks and99 broader checks across13 targets. The actual original
zero-HP warning paints after manual re-show while IsRunning stays false;
CreateLoadingTextureSet rotates with an empty motion channel and pauses under
its hidden owner. Physical original button/mask callbacks, clipped affine UVs,
fades/channel callbacks, retirement, map pings and private slot clocks remain
covered. The old effect tests now explicitly paint and retain native handoff/
hidden-clock assertions; the atlas and widget-channel test paths stay distinct.

Both aa-dev `--shot-after 8` captures were inspected:
shots/aa-ui/native-effect-clock-zero-720-20261018.png
(1280x720/default0.85, **175.7-215.2 FPS**) and
shots/aa-ui/native-effect-clock-paused-1080-20261018.png
(1920x1080/default1.0, **179.0-190.8 FPS**). The first shows the real zero-HP
manual warning plus original active loading/autoattack/toggle art. The second
shows recovered health, no warning and the hidden paused loading owner.
Both report zero script errors/WARN/ERROR and self-close (PIDs31140/24744);
the copied executable was removed. Original Lua, app APIs and native-selector
activation are unchanged.

## Ordinary drawable owner-layer flags (2026-10-18)

EnableDrawables39264d50 and DisableDrawables39264da0 use the typed string
parser392674c0, lookup3922c120 and byte setter39277bb0. They change the owner's
four layer flags at +108/+128/+148/+168, initialized true by3927ec30. They do
not change individual drawable shown flags, button state, input, child widgets
or dynamic layer factors. The previous implementation changed shown flags of
non-state art and skipped bound button backgrounds; it therefore both revived
independently hidden art and failed to hide an original button skin.

The argument must be an actual string, with zero results on every path.
Missing/nil/bool/number/table and a failed widget cast are inert; extra
arguments are ignored. Native PE table39ac6aa0 names background, artwork,
overlay and overoverlay, compared by case-sensitive strcmp through the first
NUL. Every unknown name, including empty, uppercase, highlight and all, selects
all four flags. No-argument calls do not mean all. Widget.drawable_layers
retains all four flags; rendering currently supports the first three layers.
Fourth-layer creation/ordering is a separate boundary.

Native layer draw3927af50 checks this byte before calling any contained
drawable. Ordinary owner lists now use that gate, including bound state art
and later-created art, while preserving each drawable's own visibility.
Generic Effect painting and repaint requests respect the same owner-layer
admission, so a disabled layer pauses its clock without pausing child owners.
Widget text/content and private ordinary-slot cooldown/completion draw paths
keep their existing admission. No selector, timing or application hook changes.
The separate WithChildren3927b610 route changes recursive dynamic factors;
its unimplemented behavior is not represented by these owner flags.

Ignored evidence drawable_layer_oracle.py and
native-drawable-layer-oracle-20261018.json contain **32 actual native typed
groups**, executing the thunk, parser, bindings, lookup, setter and four layer
constructors. Lua stack/result, host cast and external strcmp are controlled
endpoints; static names come from the mapped original PE. No flag mutation,
layer selection or child/factor handling is mocked. The previously executed
generic Effect paint-admission oracle supplies the layer-disabled clock control.

Before the fix, direct DisableDrawables('background') on an unchanged original
CreateEmptyButton/ApplyButtonSkin left all three skin quads painted. The test
now passes, including a physical click while the skin is gated and restoration
by unchanged common.alb ButtonInit. That exported helper uses the same enable
route as the local InitButton in button.alb; no fake InitButton global is added.
The actual SetItemIcon -> SetItemIcons -> SetIconBackGround refresh also enables
only background, preserving independently hidden background art and a disabled
artwork layer. These are controlled original-widget fixtures, not a claim that
an observed live script already invokes the direct disable case.

Crate/example checks and **143 selected checks pass**, with one existing
optional font check ignored: five new layer tests plus original state colors,
checkbuttons, text colors, masks, generic clocks, animation, private cooldown/
completion, fades, frame traversal and popup lifetime checks. Tests also retain
non-default dynamic factors, independently owned child art/text, late creation,
zero arity and native case/NUL/all argument goldens.

Both native_drawable_layers aa-dev captures were inspected:
shots/aa-ui/native-drawable-layers-off-720-20261018.png
(1280x720/default0.85, **436.2-583.7 FPS**) and
shots/aa-ui/native-drawable-layers-restored-1080-20261018.png
(1920x1080/default1.0, **397.7-597.3 FPS**). The first hides the original skin/
icon layers; the second restores them while manually hidden art stays hidden.
Both assert a physical original button click, report zero script/WARN/ERROR,
self-close (PIDs27216/46216), and leave no copied QA executable running.

## Owner overlays follow descendant content (2026-10-18)

Native Widget3927b010 paints owner background/artwork, its content virtual,
children through3927afd0, then owner overlay. Both clipped and ordinary
branches retain that order. The previous flat renderer painted each owner's
overlay before moving to its children, leaving original stack-counter labels
above overlays that should cover them. Actual unchanged cooldown_action_button
SetStack(42)/SetOverlayColor('black') reproduced counter glyph141 after
overlay137 before the fix. The script's black overlay has alpha0.75; its
ProcessDurabilityColor also supplies the original red/yellow overlay paths.

The original factory's name table identifies cooldownactionbutton as enum21;
case392a617b calls constructor392844b0, whose final VT399c6170 uses
layer virtual+13c3927af50 and content virtual+154392815f0. These are static PE
table/call/MOV/vtable facts, recorded separately from executed order evidence.
No application skill eligibility or item metadata is inferred from them.

Painting now has a content pass before descendants and a closing overlay pass.
Physical hit/input order remains Tree's existing traversal. Each pass retains
the existing transform and clip processing; an empty overlay pass avoids the
widget clone. Generic Effects advance once in the pass for their actual layer,
including alpha-zero admission. Private ordinary-slot cooldown/completion
work stays once in content. This adds no clock formula, selector, callback,
input policy, clip geometry or fourth-layer creation behavior.

Ignored widget_draw_order_oracle.py and
native-widget-draw-order-oracle-20261018.json contain **11 actual native
order/admission groups plus three FPCW controls**. Widget3927b010, child
walker3927afd0, layer3927af50, alpha/clip stacks and actual shown/fading/clip
branches execute. Rectangle lookup and final content/drawable virtual calls
are controlled endpoints that only record submission order. Three levels,
siblings, hidden/fading children, alpha0, disabled layers and raw child clip
admission controls are included. No ordering decision is mocked. The emulator
default FPCW0x0000 is explicitly recorded; PC24/53/64 controls0x007f/0x027f/
0x037f give identical traces. This ordering proof makes no claim about the
real runtime's FPCW or new x87 numeric goldens. The raw native fourth-layer
trace and child clip flags are evidence, not newly implemented public APIs.

Crate check/example build and **196 selected checks pass**, with one existing
optional font check ignored, across23 targets. Six new tests cover original
stack RED -> GREEN/physical input, three-level/clipped/layer-disabled order,
hidden versus pending-fade admission, one Effect step per layer pass at alpha0,
unchanged transform/clip results and the background-camera split. Original
button states/text/masks, private selector/cooldown/completion, chat clipping/
alpha, channels, fades, frame traversal, casting and HP tests remain covered.
An old status-bar idle-cache test now settles its immediate hide with tick(0)
before taking the generation baseline: displayed75, running/pause/resume and
cache assertions are preserved. It never invokes rendering or effect painting;
no production status-bar behavior changes.

Both native_overlay_order aa-dev captures were inspected:
shots/aa-ui/native-overlay-order-tinted-720-20261018.png
(1280x720/default0.85, **421.9-505.5 FPS**) and
shots/aa-ui/native-overlay-order-cleared-1080-20261018.png
(1920x1080/default1.0, **368.6-447.9 FPS**). The original black/red overlays
cover the stack counters in the first; clearing restores the same counters in
the second. Both assert an actual physical original-slot click, retain original
label geometry/text and callbacks, report zero script/WARN/ERROR, self-close
(PIDs51936/1748), and leave no copied QA executable running.

## Button input during its own pending fade (2026-10-18)

The unchanged character_create_view.alb enables IgnoreInputOnFading on its
cancel, next, previous and complete buttons. The controller uses its own
button Show transitions, including Show(true,1000). A physical original Next
click reproduced the missing method: the toolkit advanced Step1 to Step2
while the button's own fade was pending. It now remains on Step1 and the
same original handler advances after completion. The fixture uses an actual
hide/show transition; repeating Show(true) alone retains native no-op behavior.

Registration39230438 selects thunk3922f600/parser3922f1b0. Binding3922d410
uses the actual host virtual+14 button cast and writes byte+590. Initialization
392837b2..39283813 clears that flag. Script bools and real numbers are accepted,
including double tiny/nonfinite nonzero values; missing/nil/string/table arguments
and failed button casts are inert. Extra arguments are ignored and every path
returns zero results. The setter neither changes state/visibility nor restarts
or cancels the manager fade.

Native mouse down39283a80, up39283c30, click39280ff0 and drag start392820e0,
stop392807d0 and receive39281350 guard on own+590 AND own pending-fade+e0.
The toolkit checks these entries against ButtonData and the existing manager
pending flag. It does not consult ancestors or alpha-channel activity. The
hit function3927c6d0 remains unchanged: a fading-in button retains the hit
and blocks controls behind it. Hover39284030, focus39284090/392840b0,
wheel and generic key delivery remain live; global MOUSE_DOWN/MOUSE_UP are
also preserved. This is not a blanket handler/input gate. The separately
guarded native OnButtonDown/OnButtonUp/OnEnterPressed paths were audited,
but this pointer/drag slice adds no new button keyboard-activation mechanism.

Each guard is read once on entry. In particular, an admitted OnMouseUp
callback can start a fade and its release state continuation still executes;
the following distinct click entry then observes the new pending flag. An
admitted checkbutton click still finishes its toggle after its callback starts
a fade. Drag recipient/source guards remain independent of the existing
unconsumed-held-item cleanup. No capture, hit-order or generic keyboard
policy is added.

Ignored button_fade_input_oracle.py and
native-button-fade-input-oracle-20261018.json contain **119 actual native
groups**. The thunk, parser, Cry GetParam/conversion, binding, class casts,
initialization block, input entries, state writes and hit traversal execute.
Lua primitives/results, script delivery, capture/timing/sound and the fixture
rectangle provider are explicit endpoints. Controls include an actual release
callback writing pending=true followed by a rejected click. FPCW037f is set
explicitly, with PC24/53/64 branch controls007f/027f/037f; no runtime FPCW
or new numeric interpolation claim is inferred. Disassembly is archived as
native-button-fade-input-source-20261018.txt.

Crate/example check, example build and **195 selected checks pass** across24
targets, with one existing optional font check ignored. Eight new tests cover
the unchanged full original creation controller, typed/default/failed-class
rules, own versus ancestor fade, alpha0, immediate pending completion,
late option changes, hover/focus/wheel/keys/global events, reentrant press/
release/check continuations, drag admission and retained manager cleanup.
Existing original skin/state/text, checkbutton, popup/attachment, chat drag,
edit/keyboard, resize, cooldown/selector/completion, effect/layer/order,
channel/fade/frame and casting/HP regressions also pass.

Both native_button_fading aa-dev captures were inspected against the original
creation reference controls: shots/aa-ui/native-button-fading-active-720-20261018.png
(1280x720/default0.85, **192.2-276.3 FPS**) retains Select Race after the
blocked physical Next; shots/aa-ui/native-button-fading-ready-1080-20261018.png
(1920x1080/default1.0, **248.9-345.4 FPS**) reaches Customize after the
same control resumes. These examples render the original 2D UI only; no
character actor is supplied. Both report zero script/runtime WARN/ERROR,
self-close (PIDs43704/16480), and leave no copied QA executable running.

### ListCtrl rows by key (2026-10-19)

`ListCtrl:InsertData(key, col, text)` takes a key the script chose (a portal id such as 243 / 4096, a mail id,
`itemInfo.idx`), not a row number. `ListCtrlData.keys` holds the key per row (`ListKey`: number or string):
a new key takes the lowest free row (a full list ignores it), a known key updates its row, `DeleteData(key)` frees the
row (clears its data and key), `DeleteAllDatas` frees them all. Scripts that pass 1..n in order still land on row n.
Selection stays by row (`Select(row-1)`, `GetSelectedIdx()` is the 1-based row, the scripts index their own list with
it). Also added: `ClearSelection` (reports `OnSelChanged(0)` when something was selected, which the scripts guard with
`if selIdx == 0 then return`), `DeleteRows` (the client's name; same as `DeleteAllRows`, also drops keys and selection)
and `DeleteDataByIndex` (row based).

Call sites: only `x2ui/portal/portal.lua` (direct `listCtrl:InsertData(portal_id, 3, ...)` and `DeleteData(portal_id)`)
and `premiumservice/benefit_view.lua` / `usertrial/bounty_bullet.lua` (sequential indices) hit the native method; the
other `InsertData` users (achievements, expedition, friends, mail, loot, ...) call the `pagelistctrl` / `scrolllistctrl`
Lua wrappers, which keep their own key table and call the native one with `dataCount` (1..n in order).

Tests: `tests/listctrl_keys.rs` (6). In game (account `bevytest`, the book's `X2Warp` lists faked with `--portal-lua`
because `bevyportal` was online in another session): `shots/aa-ui/listctrl/after_book_rows.png` (Default Site, 3 rows),
`after_book_recorded_tab.png` (Recorded Site, 10 rows, page 1/2, row 3 selected, buttons enabled), against
`before_book_empty.png` (empty rows); `achievement.png` (Summary with the recent list, unchanged). fps 75-100.

### Window-position persistence callback recovery (2026-10-23)

Recovered the inherited `src/lua/drag.rs` / `tests/drag.rs` changes and validated
the actual original-script save path. Native evidence corrects the inherited
"only if visible and final rectangle differs" guard: Lua binding `392651e0`
calls `39278ab0` on its receiver regardless of capture, visibility or movement.
The stop clears the manager capture through `392a6a10(0)`, resets the cursor,
then invokes receiver virtual `+138`. Script chat `395129b0` sends event 31;
table `39ac6ab0[31]` names it `OnMovedPosition`. Manager cleanup `392aa910`
calls stop only for a captured widget. Consequently an original title-bar
click sends this callback even without movement, while an ordinary chat-tab
click (no capture/explicit stop) does not. Repeated explicit stops each notify;
an explicit stop on another widget cancels capture and notifies that receiver.

The toolkit now distinguishes the existing pointer-release entry from the Lua
method without changing public signatures. Automatic release consumes capture
and notifies once, including outside the OS window and a move returned to its
starting rectangle. The title's explicit stop clears capture before the later
release fallback, preventing duplicate callbacks. Capture and its move image
are cleared, and the UI borrow is dropped, before Lua runs. A handler can read
final bounds, change anchors, retire its window or start a new capture. Existing
retirement removes capture without inventing a movement callback. The existing
hidden-resize cancellation also completes through the callback rather than
silently dropping capture; its visibility cancellation policy is retained,
not newly claimed as a fully audited native manager policy. Handler errors do
not restore capture or retry delivery on another release.

Evidence: ignored `data/extract/aa-ui/moved_position_oracle.py`,
`native-moved-position-oracle-20261023.json` and
`native-moved-position-source-20261023.txt`. Eight offline groups execute the
actual binding, stop, capture setter and script-chat virtual: matching/no/other
capture, hidden and unchanged bounds, repeated stops, a callback installing a
new capture, and failed host cast. Host/class casts, GUI lookup, pointer query,
cursor reset, final script delivery and base-chat continuation are controlled
endpoints; callback selection and capture clearing execute unchanged. The JSON
records the read-only original PE SHA-256. Source also records the manager stop
and retirement sites; those manager sites are disassembly evidence, not extra
executed groups. No live client or local configuration is modified.

`tests/window_bounds.rs` runs unchanged pak bytecode for `ui_management.alb`,
`chat/chat_window.alb` and their ordinary World-stage dependencies. The actual
chat `OnScale` installs `AddUISaveHandlerByKey`, native first-tab movement or
border sizing reaches `OnMovedPosition -> SaveBound -> X2:SetCharacterUiData`,
and the original `ApplyLastWindowBound` restores a newly created window. Tests
check `ui_bound_chatWindow[0]`, final x/y/width/height, screen resolution, one
completion save before the next layout tick, and no save from a plain chat-tab
click or another release. The following tick can independently save through
the original `OnBoundChanged`; it does not repeat movement completion. Only the
application-owned X2 storage boundary is
an in-memory fixture: on-disk persistence, character isolation and account
switching remain aa-client's separate responsibility/tests. No new integration
hook or aa-client change is required.

Validation: `cargo test -p aa-ui --lib --test drag --test resizing --test
window_bounds --test chat_lock --test chat_reset --test chat_lifecycle --test
popup_lifecycle --test button_fading --test event_handlers --test widget_frame`
passes **108 tests**, with the existing optional font-metrics test ignored.
Seven recovered/new drag regressions cover arity/receiver identity, duplicate
release prevention, explicit stops, outside release, returned motion,
reentrancy, retirement, hidden sizing and expected handler failure; two
original-script regressions cover persistence. Only aa-ui targets were built,
using the shared target directory. No drawing, movement geometry, visual
example or GUI process changed in this behavioral slice, so no new PNG/FPS
claim is made; no copied executables or temporary native binaries remain.

Limits: in the original-chat fixture the message child wins hit testing over
the side/bottom resize strips; the acceptance test uses its exposed top border.
Child-versus-owner resize arbitration needs its own native input audit.
Secondary-tab splitting/cross-window reparenting remain the previously recorded
chat boundary. Native `OnChangedAnchor` dispatch is still absent; the existing
tick-based `OnBoundChanged` path is unchanged. This slice closes completion
notification and original-script bounds persistence, not those toolkit gaps.

## Native OnChangedAnchor dispatch (2026-10-23)

`OnChangedAnchor(self)` is native event **32**, with no additional arguments.
The script-chat VT `399e7400 + d4` points at `395129d0`, which calls
`3927b8b0(32)` before its chat continuation `392ca030`. Event-name table
`39ac6ab0[32]` names it `OnChangedAnchor`. Actual `3927b8b0` and
`39279ae0` marshalling push one ScriptAnyValue(type 6), the receiver's script
table. This is a synchronous native setter/propagation notification, not a
rectangle comparison in the later `OnBoundChanged` tick.

`39276dd0` installs/replaces an anchor, rejects dependency cycles through
`39275e70`, and calls `39276350(1)` even for an identical anchor. Its final
sixth argument is not consulted by this body. `39276940` / `39276a70` remove
all anchors and dependency records without emitting the event. Replaced
points retain their old target dependency record until removal: changing the
old target still propagates, even if the current point targets another widget.
The retained records also participate in native cycle rejection.

`39276350` resolves its source, then walks its **live** dependent vector
(`+44..48`) in registration order before invoking source virtual `+d4`.
The tree relation is independent of widget parenting and visibility. New
target installation invalidates downstream records via `39275ec0`; a node
whose targets are not all ready is skipped by `39275f10`. Existing-target
updates do not perform that invalidation. A diamond therefore sends one
downstream callback on the first new-target propagation, but can send two on
an existing-target update. Native integer-coordinate controls expose width
99 followed by 100 in those two callbacks, before returning to final width
100. The toolkit preserves these intermediate target bounds during dispatch,
without replacing its ordinary final layout/cache outside dispatch.

Native width/height/extent bodies `392763c0`, `39276410`, `39276460` propagate
even for unchanged values. Bindings `39261740`, `392617c0`, `39261840` read
their notification control at Lua parameter **2**. For SetExtent this is the
height, not a third flag: zero height suppresses notifications, and a third
false does not suppress a nonzero-height extent. Actual Cry GetParam
`3550a460` / conversion `3550bf90` execute in the oracle; bools and numeric
zero/nonzero control delivery, while failed optional conversions retain true.
Silent size setters still resolve/propagate bounds, including during another
handler. Required numeric setter arguments and invalid anchor targets/pairs
are rejected before a notification can be invented.

The existing implemented anchor, size, drawable and status-bar child-anchor
setters now use this dispatch. Native `39278a10` installs the absolute anchor
before manager capture; movement/resize continuation `3927a4ab` propagates
after its geometry writes. Capture start and subsequent admitted toolkit motion
now dispatch, retaining dependency readiness during motion. Native scale
`3927d120` / `3927b2f0` first recurse through owned children and propagate
their anchors, then propagate the source before OnScale. The existing explicit,
started, inherited and stepped scale entries use that same callback ordering.
The source's native `+7c` readiness override remains active through its anchor
callbacks, including a reentrant scale while another target is pending, and is
cleared before OnScale. Alpha-only changes do not acquire an anchor notification.

No Ui borrow survives into Lua. Reentrant setters run before the outer setter
returns; live handler replacement, self-unanchoring and appended dependents are
observed. Retirement detaches incoming/outgoing anchor records and notifies
surviving dependents (`39276a80` / `39276750`), without later dispatch to dead
objects. A handler error does not enqueue a retry. Native active-vector
invalidation is not reproduced as unsafe memory access: toolkit lifetime and
vector bounds checks remain. The append oracle explicitly supplies spare
capacity; it does not claim native safety after iterator-invalidating growth.
Composite retirement retains the toolkit's existing batching; the executed
detach controls do not prove every composite-destructor interleaving.

Evidence retained, git-ignored, under `data/extract/aa-ui/`:
`changed_anchor_oracle.py`, `native-changed-anchor-oracle-20261023.json`, and
`native-changed-anchor-source-20261023.txt`. **40 offline native groups**
execute anchor installation/removal, readiness, propagation, size bodies and
optional controls, event marshalling, reentry, stable live-vector mutations,
detach and scale propagation. Allocator/free/memmove, script VM primitives,
host/class lookup, final clip-cache geometry and chat-specific continuations
are explicit endpoints; callback admission/order and anchor geometry execute.
The original x2game SHA-256 is
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`;
the JSON also records the original CryScriptSystem hash. FPCW is explicitly
037f; no real-runtime FPCW, fractional-layout precision or live-client claim
is inferred. No Ghidra project or original client/configuration file was opened
for modification.

`tests/changed_anchor.rs` covers synchronous identity/arity, no-op and rejected
setters, silent controls, hidden/nonchild and diamond ordering/intermediate
bounds, retained targets/cycles, reentry, live mutations, retirement, handler
failure, drawable dependencies, scale order and captured parent movement.
`tests/window_bounds.rs` still runs unchanged original World-stage bytecode:
its actual OnChangedAnchor invokes SaveBound and ProcChangedAnchor before a
layout tick, without setting isMoved or inventing OnMovedPosition. Repeating
an anchor/extent still saves a previously moved window. Original
ApplyLastWindowBound restores the saved rectangle and now produces its two
expected synchronous saves (anchor, then extent). Only the X2 storage boundary
is an in-memory fixture; account/disk persistence remains owned by aa-client.

No public API, integration hook, dependency, manifest, or aa-client change is
added. The existing tick-based OnBoundChanged behavior remains unchanged.
Child-covered resize arbitration and secondary-tab splitting remain separate
gaps; this is not a claim that the whole UI is complete. This behavioral change
has no new visual example/window, PNG or FPS measurement.

Validation: `cargo test -p aa-ui --lib --test changed_anchor --test
window_bounds --test drag --test resizing --test chat_lock --test chat_reset
--test chat_lifecycle --test popup_lifecycle --test button_fading --test
event_handlers --test widget_frame --test widget_channel --test
animation_inherit --test screen_bounds --test statusbar --test natives` passes
**175 checks across 17 targets**, with the existing optional font-metrics test
ignored. After the final capture-baseline/cache fix, the **37 affected checks**
in changed_anchor, drag, resizing and window_bounds pass again. Twelve new
dispatch regressions plus the additional original-chat persistence regression
are included. Only aa-ui targets were selected, using the shared target
directory. One `cargo clippy -p aa-ui --lib --test changed_anchor --test
window_bounds` run exits successfully, retaining 27 warnings in unchanged
aa-ui code (plus the existing aa-dev dependency/aa-client manifest warnings).
Its output is retained in ignored
`data/extract/aa-ui/changed-anchor-clippy-20261023.log`. No automatic lint fixes
or other crate changes were applied.

## Child-covered chat resize borders (2026-10-23)

The original World/chat fixture now resizes through its message-covered left,
right, bottom and bottom-corner borders. Hit priority remains with the child.
Native script-chat hit virtual `39513d00` visits children before considering its
own border; script-message `395126c0` also visits its children first. Generic
`3927c6d0` retains reverse child order. Resizing through a message is a conditional
mouse-down continuation, rather than a parent substituted by hit testing.

Base press `3927cb20` admits automatic sizing only with resizing enabled, the
receiver enabled, physical LeftButton (1), no held cursor (`3a5a4e68 == 0`), and
a valid border point from `39277580`. It starts capture and returns before the
owner's script mouse-down. Otherwise event 14 (`OnMouseDown`) requests a boolean
result through `3927b990` / `390243b0`. Continuation at `3927cc1d` requires an
unconsumed result and no `OnDragStart` handler at `+1d4`, before calling parent
virtual `+16c`. The barrier is that handler record, not just RegisterForDrag.
Cry `3550c380 -> 3550bf90` confirms boolean/numeric return conversion: true and
nonzero consume, false/zero do not; nil and strings fail conversion and retain
false. A failed handler leaves that false default, so an otherwise eligible
base continuation can still resize; it does not enqueue a callback retry.

Class priority matters. Registered ButtonWidget press `39283a80` consumes before
that base fallback; an unregistered button can use it. Selectable textbox
`3929c1e0`, ordinary multiline-edit `39294b60`, and slider `392c2a80` retain
their own press paths. The slider's actual `+58` cast at VT `399c93c0` is
`392c1e30`; the original message constructor resolves its `slider` through this
cast at `395122a3..2bf`. Both thumb and track entries were executed. Text-position
lookup and slider value update are controlled endpoints, not a claim that their
whole selection/value algorithms were recovered in this task.

Resize predicate virtual `+118` provides a second guard. Chat `392ca3c0` rejects
hovered tab buttons and delegates to the selected message. Script-message
`39511d00` rejects its hovered up/down/bottom buttons, and rejects the native
slider's rectangle even while the slider is hidden. These references now live
in private message data, independently of script field/getter replacement.
The toolkit uses its existing mouse-over chain for those native hover flags.
AllowTabSwitch remains a separate drag policy; it is not substituted for
UseResizing, and locking does not acquire a move through this continuation.

The implemented press continuation is bounded to a covered chat border and
the audited inherited base classes (message, empty/window/label, unregistered
button). It rereads live state after each admitted handler, stops at drag and
consumption barriers, and never holds a Ui borrow into Lua. Other widget-class
continuations and generic parent bubbling elsewhere remain unchanged. It does
not turn arbitrary interactive child presses into owner resizing. Existing
mouse target identity, global mouse-event payloads, button/selection/slider
input and explicit StartSizing remain on their existing paths.

Native release has a useful distinction: the press identity is still the
message, while sizing capture belongs to the chat. Manager `392a7506` records
the press identity; `392aa410` freezes hover updates during capture. Actual
`39277da0` / `3927cc70` / click query `392a71e0` can still admit a message click
when its press/release identity matches, including after StopMovingOrSizing.
The covered-child path preserves this; it does not invent sticky click
suppression after cancellation. Its release requires the same hit identity,
so a different button under the message cannot acquire that click after an
explicit stop. Existing direct-owner border click behavior is unchanged.
The toolkit's general hover/capture traversal is unchanged; the oracle supplies
the retained native hover/press identities. It records terminal click dispatch;
full link activation after a resize is outside its claim.

Existing capture completion stays native: stop `39278ab0` clears capture through
`392a6a10`, then receiver virtual `+138` (script chat `395129b0`, event 31).
Another receiver's explicit stop cancels the chat capture and notifies that
receiver. Completion can install another capture. Retirement clears owned
references without movement completion (`392aa7e0`, retained disassembly).
Disabling automatic resizing during capture does not cancel existing sizing.
The toolkit's previously documented hidden-sizing cancellation remains;
this task does not claim a new audit of every native visibility/capture policy.

Evidence retained, git-ignored, under `data/extract/aa-ui/`:
`chat_resize_input_oracle.py`,
`native-chat-resize-input-oracle-20261023.json`, and
`native-chat-resize-input-source-20261023.txt`. **49 fresh offline native groups**
execute hit walks, eight border points, admission/control/handler barriers,
class press priority, boolean-return conversion, release/click admission,
explicit stops and callback capture replacement. Geometry/transform providers,
GUI/focus/press bookkeeping, final script/click delivery, button art/timing/sound,
text-position lookup, slider value update and Lua primitives are explicit
endpoints. Capture installation's anchor work is an endpoint followed by the
actual capture setter; the prior anchor/motion audit is not rerun or expanded.
The original x2game SHA-256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`;
the JSON also records CryScriptSystem's hash. FPCW is explicitly 037f. No live
client/configuration, database or original asset was modified; no Ghidra project
or copied native executable was created.

`tests/window_bounds.rs` runs the unchanged original World/chat bytecode and
native save callbacks. Five directions use the authored overlapping message;
the three exposed top directions use a synthetic base child of that original
chat. All eight press/drag/release directions reach final bounds, one movement
completion save before the layout tick, and original ApplyLastWindowBound
restore. Other regressions cover locked/disabled sizing, interior/outside/right
presses, handler returns and reconfiguration, registered/unregistered buttons,
edit text selection, textbox/slider priority, native scroll controls and tabs,
hidden slider admission, explicit/other-receiver stop, disable/hide/retirement,
retirement during child dispatch, completion starting another capture, and a
different interactive descendant receiving release after an explicit stop.
The X2 storage boundary remains in memory; account/disk persistence is aa-client's
responsibility. No public API, manifest, dependency or integration change is added.

Validation: **72 checks across seven focused targets pass**: button_fading (8),
changed_anchor (12), chat_lock (7), drag (18), popup_lifecycle (9), resizing (5)
and window_bounds (13). The final window_bounds/resizing/popup_lifecycle run
passes all **27** checks after the final covered-child release-identity guard.
One `cargo clippy -p aa-ui --lib --test window_bounds --test resizing` run succeeds
with 27 warnings in unchanged aa-ui code, plus the existing aa-dev dependency
and aa-client manifest warnings. Its log is retained in ignored
`data/extract/aa-ui/chat-resize-clippy-20261023.log`. That single lint run preceded
the final small release-identity guard and its regression; the final checks
above followed it. No second clippy run or automatic lint fixes were applied.
The earlier
175-check selection, optional font test and 40-group anchor oracle were not
rerun wholesale. All Cargo commands selected aa-ui and used the shared target.
This is behavioral validation; no new visual window, screenshot or FPS claim
is made. Secondary-tab splitting, general OnBoundChanged timing, other native
class fallback configurations and whole-chat parity remain outside this slice.

## Native GetTextLength unblock (2026-10-23)

`editboxmultiline:GetTextLength()` and `textbox:GetTextLength()` now return the
current native text-buffer length as a script number. Native registrations
`3923b820..b87a` and `3925ca98..caef` bind `3923b270` (multiline scalar-count
field) and `3925c410` (textbox text virtual, then imported XlStringLen at
`38ff6c5c`). Their host casts reject incompatible receivers. The captured
native class method lists expose this getter on those two classes; ordinary
single-line `editbox` has no such native getter. No class surface is expanded.
The implementation reads the buffer directly, including inside synchronous
OnTextChanged; it does not invoke a replaceable Lua GetText or synthesize a
digit fallback. Textbox length stops at its native C-string terminator.

The parent-owned read-only `tools/user_music_oracle.py` executes xlcommon
`3301b000`: `é도😀` is three scalars and `e` plus combining acute is two.
This task retains the getter registrations/wrappers in ignored
`data/extract/aa-ui/native-text-length-source-20261023.txt`. The focused
`tests/text_length.rs` checks both classes, Unicode, immediate change-handler
visibility, direct buffer access and incompatible receiver rejection. This is
an implementation unblock for the original composer bytecode; the parent's
composer integration tests and captures remain parent-owned.

Validation: cargo test -p aa-ui --test text_length passes its focused check. No clippy run is used for this unblock; the single final lint run remains reserved for the tab work. No visual claim is made for this getter change.

## Native secondary chat-tab splitting and window drops (2026-10-23)

A physical LeftButton drag of a secondary original chat tab now creates an
independent chat window. It transfers the existing button and message frame;
scrollback, filters, palette, handlers, original raw ID and Lua table identity
survive. It does not re-run OnCreateChatMessage, OnCreateChatTabButton or
OnSetChatTabId. Ordinary clicks still select, locked drags still notify, and
first-tab drags still move the whole window. No original Lua behavior is
replaced and no original X2 method/event is invented.

Native button event4 reaches `392cc600`: its secondary branch calls
`392cc4a0`. Factory `39512e90` requests `chatwindow/dup` with the old parent
and source widget. Base `3927a9f0` and chat `392cb742..392cb8ac` copy plain
configuration before the original OnCreateChatWindow callback. This toolkit
copies supported extent, base alpha, layer/enable/resize configuration and
chat layout/notification configuration; it retains fresh native children,
handlers, input state and rows. Original callbacks may override those copied
settings. Native `39512e90` then enables tab switching. `395142f0` allocates
an empty model window and calls original OnDuplicatedChatWindow(widget,id),
which installs the original window-save handlers.

Detach `392cae10` removes the row/child relationships, forces selection of
max(oldIndex-1,0), then virtual `39512830` moves its metadata into native
pending storage. Insert `392cc2a0` reparents the SAME objects, reanchors all
content, selects/binds the destination input raw ID and fires selection
callbacks before `39512c30` attaches/saves metadata. The new window is placed
at the former tab's top-left, shown, and captured for ordinary movement.
The existing movement-completion callback saves its final bounds. Script
OnDragStart precedes this internal start; script OnDragStop precedes native
internal release event5. Live locking is reread after those handlers.

Release `392cca60 ->392cc8c0` scans the native window creation list for an
unlocked window whose internal tab-strip rectangle contains the cursor.
That rectangle spans the entire window width, including tab-area insets;
its stored vertical anchors supply its height. It does not substitute the
generic mouse target or require the pointer to be on a particular button.
The first matching window wins, including the source itself. A source
containing raw ID0 cannot transfer all its rows (`392c9790/39513080`). The
nearest destination button is selected by squared center distance; ties keep
the earlier row, and equality at its horizontal center inserts AFTER it.
Insertion before an existing raw-ID-zero row is refused; appending is legal.
Secondary-start splitting itself has no raw-zero predicate, as the executed
adversarial raw-zero-secondary control confirms; no additional tab rule is
invented. Native bulk `392cbef0` detaches ALL rows/model records while the
source's original row vector and selection remain observable, then clears
source selection to -1. `392cc430` inserts/selects/attaches rows in order;
the final inserted row is selected. An emptied configured window retires at
the next toolkit update, preserving transferred children.

### Application-owned configuration and failure handling

Four OPTIONAL AAUI hooks integrate the parent's authoritative configuration:

| Hook | Contract |
|---|---|
| `ChatAllocateWindow()` | Return an allocated empty window ID 0..7, or nil on failure. Native chooses the lowest free ID. Configure extra selected 0/method 0 and unlocked state in the app. Without this hook only, the standalone toolkit chooses the lowest ID absent from live windows. |
| `ChatTabDetached(windowId,rawTabId)` | Runs AFTER forced source selection/ChatSelection/OnSelectedChatTab during a split; bulk detach has no source-selection callback. Remove the raw ID from source order, preserve metadata/history pending. |
| `ChatTabAttached(windowId,rawTabId,zeroBasedIndex)` | Transfer: runs AFTER physical insertion, input binding, ChatSelection and OnSelectedChatTab. Same-owner release reorder: runs AFTER paired-vector/layout change, with no Detached/ChatSelection/input rebind. Insert or reorder the owner/order record and save while retaining metadata/history and numeric selection. |
| `ChatReleaseWindow(windowId)` | Idempotently release an allocated still-empty duplicate on an aborted split. Never delete a tab. |

No UI borrow survives into these hooks or original callbacks. The parent
owns account/disk persistence, raw-ID metadata and method ownership; it must
retain detached metadata and avoid resolving a pending raw ID back to its
former live window. Bounds still flow through unchanged original Lua/X2
storage handlers. This task changes no aa-client, root manifest/dependency,
README or other crate.

The original allocator `39505ca0` searches 0..7. Its full case returns false
to the VOID `395142f0` callback, yet the outer native split still transfers
into an unconfigured duplicate with ID-1. That native data-loss bug is NOT
reproduced: nil/error allocation retires the fresh empty duplicate without
removing the source row. The allocator hook must commit plain state atomically
before returning an ID, and undo its own changes if it throws or cannot save:
an unreturned ID is unknowable to the toolkit. Lua errors in original handlers
log and continue; no retry is scheduled. Notification hooks likewise log errors
and retain native UI continuation, so app detach/attach hooks must make their
synchronous plain-state mutations reliable; these hooks are not an ACK protocol.
A Rust callback panic is not an application transaction and must be unwound or
rolled back by the callback owner; no catch_unwind recovery is promised.

Live ownership is checked after callbacks. If the destination retires before
detach, the empty allocated duplicate is released. If it retires after detach
while the original row objects/source survive, the toolkit restores the source
row at the old index (clamped for external row changes), selects it through the
usual callbacks, then calls ChatTabAttached(source,rawId,index) and releases
the empty duplicate. If both owners retire or a callback destroys the row
objects, physical rollback is impossible: the toolkit retains manager history
and retires surviving orphan objects. The parent's detached metadata remains
pending with its recovery owner for later rehydration. Rehydration keeps fresh
native scroll-control references rather than stale controls from the retired
history frame. Arbitrary callback-driven deletion/reallocation and external
panics are not a general cross-layer transaction guarantee.

### Evidence and bounded validation

Retained ignored evidence under `data/extract/aa-ui/`:
`chat_split_oracle.py`, `native-chat-split-oracle-20261023.json`, and
`native-chat-split-source-20261023.txt`. **32 offline native groups** execute
actual duplicate wrappers, split detach/insert/select, release target traversal,
bulk transfer, nearest/side math, raw-zero/locking guards, the allocation-search
prefix and the 24-field chat clone-copy block. The full-allocation bug is an
explicit executed control. Factory/object allocation, vector storage primitives,
child/observer operations, transform/center providers, Lua delivery, model maps,
saves, input rebinding and capture are explicit endpoints. It is a control-flow
and data-operation oracle, not a live-client capture or full runtime emulation.
The original x2game SHA-256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`;
FPCW is explicitly 037f. No original client/configuration file or RE project
is modified.

`tests/chat_split.rs` combines toolkit ordering/error/retirement regressions
with the unchanged original World/chat bytecode and meaningful physical
press/drag/release. It checks split placement, original save callbacks,
rejoining onto the original raw-zero window, deferred empty-window retirement,
retained messages/handlers/objects, callback locking, full allocation, callback
overrides of cloned configuration and pending-history recovery.

This is the original bounded split/reparent checkpoint. The later drag-preview
section below implements native caret/overflow sliding and release-time row
reordering, and the bound-change audit below supersedes the event33 limit.
Other widget-class press fallbacks, hover traversal and link activation retain
their separately documented limits; this is not a whole-chat parity claim. The checked-in
real-client spawn reference is used to inspect original HUD/chat art and text;
there is no supplied real-client split screenshot or new live-client capture.

Validation: `cargo check -p aa-ui`, `cargo build -p aa-ui --example
native_chat_split`, and **84 checks across ten focused targets** pass:
button_fading (8), chat_alpha (6), chat_lifecycle (9), chat_lock (7), chat_method (5),
chat_split (12), drag (18), resizing (5), text_length (1) and window_bounds (13). The test
output is retained in `data/extract/aa-ui/chat-split-tests-20261023.log`.
One final `cargo clippy -p aa-ui --lib --test chat_split --test text_length`
succeeds, with 27 pre-existing aa-ui warnings, the existing aa-dev warning
and aa-client manifest warning; no new-file warning or automatic lint fix.
Its log is `data/extract/aa-ui/chat-split-clippy-20261023.log`. All Cargo
commands selected aa-ui and used the shared target directory; lock waits
were allowed to finish.

The aa-dev example `native_chat_split` retains the matched 1920x1080 captures
`shots/aa-ui/native-chat-split-before-20261023.png` and
`shots/aa-ui/native-chat-split-after-20261023.png`. The BEFORE capture runs
the same physical drag against the prior toolkit and logs one window; AFTER
logs two and the same button's new parent. Both PNGs were inspected: the old
All frame retains all three lines, and independent Harbor retains its two
filtered lines, original tab/add art and unchanged text styling. The source
window stays in place. Both runs report zero script errors and 43 unchanged
World-fixture stubbed natives, with no runtime warning/error. The final
AFTER process records exit 0 after saving its screenshot. Logged FPS is
**263.1-389.9 before**, **196.8-244.5 after**; other builds/sessions were active,
so these are capture observations, not an isolated performance comparison.
Logs are `data/extract/aa-ui/chat-split-before-20261023.log` and
`chat-split-after-20261023.log`. Temporary copied executables/debug probes
were removed; screenshots, test/lint logs and native evidence are retained.

## Native OnBoundChanged timing and propagation (2026-10-23)

`OnBoundChanged(self)` is native event **33**, distinct from anchor event32.
`39ac6ab0[33] -> 399bfb44` supplies its exact name. SetHandler registration
`39269420..39269470` binds `39264ff0`, which reaches `392792e0` (name lookup
`3922c160`) and `39279260` (handler-slot storage/replacement). Registration,
replacement and release do not notify. The executed registration control
changes only slot `+184 + 33*4`; the next unchanged-size setter calls the new
handler. Actual `3927b8b0 -> 39279ae0` marshals just the receiver table, with
no extra Lua arguments.

Width/height/extent bodies `392763c0`, `39276410`, `39276460` first call
`39276350(notify)` to finish event32 propagation, THEN call source virtual
`+d8` when notify is true. Script-chat `399e7400+d8 -> 39512990` dispatches
event33 before `392ca020 -> 3927b5e0`; generic widget `3927bd60` has the same
script-first continuation. That continuation walks the **live anchor-dependent
vector** in registration order, invoking each dependent's `+d8` recursively.
For W -> C -> G and W -> O, size-setter order is G32,C32,O32,W32,
W33,C33,G33,O33. A diamond can repeat event33 on the same descendant.
Visibility, ownership parenting, rectangle equality and anchor readiness do
not filter bound propagation. An owned child without an anchor dependency
does not receive it. A source without a handler still walks its dependents.

The same-value size calls still notify. The existing binding rule remains:
optional notification control is read at Lua parameter **2**, including
SetExtent's height; zero height suppresses both events, while a third false
does not suppress a nonzero-height extent. Failed optional bool conversion
retains true. Silent setters still update geometry/anchor readiness. Event33
can run while event32's source readiness check rejects propagation, as the
executed reentrant pending-target size control demonstrates.

Anchor install/update `39276dd0`, silent removal `39276940`, target detach
`39276a80` and scale `3927d120/3927b2f0` do not themselves invoke event33.
The retained motion tail `3927a435..3927a4e6` likewise finishes through
`39276350(1)`; completion remains its separate event31 path. Lua callbacks on
these routes may explicitly call a size setter, which legitimately emits
event33. Native drawable-base VT `399c6c90+d8 -> 3928aab0` returns directly:
it is a bound-propagation barrier, including for a widget anchored through a
drawable; anchor event32 still traverses that dependency.

`lua/bounds.rs` now implements this synchronous source/dependent walk.
`anchors::size_changed` calls it after the existing complete event32 path.
The periodic Seen-rectangle scan, first-layout notification and four-pass cap
are removed. This supersedes the earlier tick-based descriptions in this
document, including historical resize/anchor milestones. No guessed periodic
fallback, delayed retry or no-op suppression is retained. Rendering's content
comparison is independent and still suppresses unchanged mesh rebuilds.

Every callback runs without a Ui borrow. Reentry completes before the outer
walk continues; live handler removal/replacement and source-callback dependency
changes are observed. Retirement checks stop later delivery to dead objects
and retain existing composite-retirement batching. The oracle mutates a
source's dependent vector only before that source captures its iterator.
It never tests iterator-invalidating removal/growth during an active native
walk. Toolkit vector/lifetime checks remain safe; native undefined behavior
is not reproduced or claimed as defined ordering.

### Evidence and remaining scope

`crates/aa-ui/tools/native_bound_oracle.py` is a repeatable read-only offline
oracle. **48 native groups pass**, executing registration, handler replacement,
three size bindings/bodies, marshalling, anchor geometry/readiness, both bound
continuations, hidden/nonchild/diamond/no-op/silent controls and safe reentry.
Allocator/free/memmove, imported string comparison, script VM primitives,
host casts, chat layout, clip-cache and final scale geometry are explicit
endpoints. The actual control flow and callback ordering are not endpoints.
Native motion is retained source evidence, not newly emulated full input.

Ignored retained evidence in `data/extract/aa-ui/`:
`native-bound-oracle-20261023.json`, `native-bound-source-20261023.txt` and
`bound-before-20261023.log`. The before regression fails because an identical
SetWidth returns with only G32,C32,O32,W32, missing all four native bound
callbacks. x2game SHA-256 is unchanged:
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`;
CryScriptSystem is
`75ad3b0e7b483b12a113fd6cb28bcab785259f28bca145cb3f28e0fa8e72f162`.
FPCW is explicitly 037f. No live-client/configuration/database/original asset,
Ghidra project or copied native executable is modified or created.

`tests/bound_changed.rs` checks the ordering, no-op/silent/invalid controls,
registration/first-frame negatives, hidden diamonds, retained dependencies,
parent extent, reentry/readiness, live handlers, drawable barriers, movement,
scale, errors and retirement in either phase. `tests/original_bound_changed.rs`
runs unchanged original World/ui_management/general-skill bytecode: real save
closures observe native source/dependent order before the setter returns;
original general-skill columns reflow synchronously; movement/scale use the
original anchor/completion saves; reset inside the original bound continuation
has no stale dispatch. The World harness retains its existing fake game data;
within the save callbacks, X2 storage and Proc* observation are fixture
boundaries, and no original handler is replaced. Existing original-chat
bounds/restore tests now expect anchor + extent-anchor + extent-bound saves
(three synchronous saves).

This slice proves general setter and registered dependency delivery. It does
not newly audit every widget-class internal geometry continuation, tab/grid
synthetic anchor construction, automatic text sizing or raw host Tree writes.
Those callers must acquire their own proven native mutation/notification path;
there is no periodic callback to mask missing class behavior. Root screen
change integration, hover/class/link expansion, native unsafe-vector behavior,
live-runtime floating precision and whole-client parity are outside this
claim. No public API, dependency, manifest, aa-client integration or f6f0929
hook-contract change is required. This is a behavioral deliverable with no
new visual harness, screenshot or FPS claim.

Validation: **225 distinct checks across 23 aa-ui targets pass**, with the
existing optional font-metrics test ignored. Targets: lib (29), bound_changed
(10), original_bound_changed (4), changed_anchor (12), chat_reset (7),
grid_bounds (4), window_bounds (13), animation_inherit (8), auto_clip (6),
auto_clip_scale (8), chat_lifecycle (9), chat_lock (7), chat_split (12), drag
(18), natives (21), popup_lifecycle (9), resizing (5), screen_bounds (3),
statusbar (11), tab_active (6), tab_custom (2), widget_channel (11),
widget_frame (10). Logs: `bound-focused-20261023.log`,
`bound-regressions-20261023.log` and `bound-final-20261023.log` in the ignored
evidence directory. The initial expanded run caught a reset-fixture omission:
the original first chat window delays save-handler registration until its
first OnScale. The fixture now invokes that original bootstrap; its four
original-Lua checks and the remaining eight targets pass with exit0. This
required no implementation change. `bound-native-20261023.log` records the
final 48-group native run. All Cargo commands select aa-ui and use the shared
target; other sessions' build locks were allowed to finish.

One final `cargo clippy -p aa-ui --lib --test bound_changed --test
original_bound_changed --test window_bounds` succeeds (exit0), retaining
27 pre-existing aa-ui warnings plus the existing aa-dev dependency and
aa-client manifest warnings. No warning points to new behavior/tests and no
automatic fixes or second lint run were used. Its retained log is
`bound-clippy-20261023.log`. New Rust tests were formatted directly; owned
`git diff --check` passes. No temporary copied executable was needed or left.

## Native chat drag preview, overflow sliding and release reorder (2026-10-23)

The original frame update does **not** reorder rows on hover. `392cb310` scans
for a dragging chat, then calls overflow repeat `392ca150` and caret preparation
`392caff0`. `SetSlideTimeInDragging` stores milliseconds directly at +30c; it is
an overflow-arrow repeat interval, not a tab animation duration. The accumulator
uses a strict `elapsed > interval`, resets even with neither arrow hovered,
and retains its remainder across pointer exits. Native `<-` / `->` buttons are
20x20 outside the left/right edges of the tab area, hidden unless
UseSlidingButton is enabled and content overflows. Each step uses maximum tab
width, with original first/end clamps. No animated tab interpolation is added.

The original script's GetCaretDrawable artwork is centered on the nearest tab's
left/right top corner plus SetCaretOffset. Center equality chooses RIGHT;
nearest-distance ties retain the first row. Both strip edges are inclusive.
Hiding retains the last anchor and side flag. Preview rejects the moving owner and insertion before raw0, but never changes
owners, rows, selection, message content or input binding. The caret is outside
the tab clipping area. Original manager preparation `39502100` freezes the
three drawable layers of chats whose native input is hidden at their OVER
values; `395021b0` restores dynamics and forces only the completing owner OVER.
The input predicate is its own shown flag (`395128e0 ->392778a0(false)`), not
keyboard focus or inherited visibility. Explicit capture stop, hidden/retired
owners and pointer release clean transient state safely.

Release dispatch `392a7a00` invokes the drag-over button's OnDragReceive before
MouseUp and source OnDragStop. Button `39281350` calls Lua event22 first, then
chat internal event16 through `392ccb30`. The same-owner branch `392cc780 ->
392cc520` moves paired button/message entries and relayouts. Its LEFT/RIGHT flag
only controls adjacent no-op guards: the insertion index is **not adjusted**
after removal. Numeric selection and the previous input raw binding stay put;
there is no native selection callback or skin callback. The approved app-only
`AAUI.ChatTabAttached(windowId, rawId, finalIndex0)` persists a real reorder
AFTER vector/layout mutation, without Detached or ChatSelection. The existing
parent adapter accepts a raw ID already owned by this window. No new hook,
public Rust API, dependency, manifest or aa-client edit is required.

A foreign receiving tab uses the existing detach/select/attach path, preserving
its exact button/message/handler objects and history, and checking protected raw0.
Whole-window release remains the separately traced creation-order strip scan
and bulk transfer from f6f0929. SetTabButtonAlpha remains its prior stub.
General held-item receive timing, generic hover
traversal, input capture replacement across unrelated widget classes and the
native unsafe vector-iterator bug are outside this bounded change. In particular,
no hover-time mutation or modern index-normalization rule is introduced.

Retained evidence: `crates/aa-ui/tools/native_chat_drag_oracle.py` executes
**32 offline native groups**, writing ignored
`data/extract/aa-ui/native-chat-drag-oracle-20261023.json` and
`native-chat-drag-source-20261023.txt`. Actual caret predicates/anchor calls,
frame dispatch, repeat/slide arithmetic, paired-vector reorder, GUI release
ordering, input-visibility predicate and manager preparation/restore traversal execute under Unicorn with
FPCW=037f against the original x2game SHA-256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
Geometry, raw-zero admission, widget/vector primitives and final Lua/model
operations are controlled endpoints. No live original client/configuration,
read-only pak, RE project or server data is modified. The retained source also
identifies the input-visibility predicate; no full native renderer claim is made.

`tests/chat_drag.rs` adds ten physical/runtime checks: original bytecode and
artwork/anchor preservation; no hover mutation; inclusive raw0 center rule;
exit/reentry, release outside the OS window, locking, cancellation and owner
retirement; original repeat threshold and last-step/reset behavior; receiving
button eligibility; paired identities/history and old input binding; receiver
before MouseUp/DragStop; foreign-owner transfer; reentrant selection/retirement;
and no repaint from an unchanged preview/repeat clock. Repeat clocks live in
DragState outside widget repaint data and are discarded on owner retirement.

The aa-dev example `native_chat_drag` retains matched original-script
1920x1080 captures:
`shots/aa-ui/native-chat-drag-before-20261023.png` and
`shots/aa-ui/native-chat-drag-after-20261023.png`. Both use the same physical
press/threshold/motion and held pointer (130,235); no release occurs before the
capture. BEFORE uses the pre-change toolkit. AFTER shows the original cyan HUD
caret at the traced insertion edge; neither capture transfers the hovering row.
Both PNGs were inspected against each other and the real-client spawn art/text
reference. There is no retained real-client drag capture, so native instructions
and the original-script caret-rectangle test establish its exact placement.
The first baseline harness's logging-only borrow error was fixed before the
matched captures. The baseline was rebuilt with saved pre-change source, then
all owned in-progress code restored; no foreign path was reverted or edited.

Harness FPS: BEFORE 228.2 / 195.0 / 265.0 (4.45 / 5.31 / 3.80 ms), AFTER
225.0 / 182.4 / 218.7 (4.51 / 5.72 / 4.70 ms). Other shared builds/apps were
active and sample loads differ; these are bounded capture observations, not
an isolated performance comparison. Both runs close via `--shot`. Logs and
native evidence remain under ignored `data/extract/aa-ui/chat-drag-*`;
matched captures remain under ignored `shots/aa-ui/`.

Final validation: **271 checks across 30 aa-ui targets pass**, with only the
existing optional font-metrics test ignored. This reruns all 23 targets from
the 225-check event33 checkpoint plus animation, attachment, chat_alpha,
chat_drag, chat_method, chat_tab_layout and listctrl_geometry. The ten new drag
checks pass on the final source, including the unchanged-preview repaint test.
The complete final log is `chat-drag-final-20261023.log`; prior focused and
expanded logs are retained alongside it. All builds select aa-ui and use the
shared target; lock waits were allowed to complete. No workspace run, alternate
target, cargo clean, foreign process termination or push was performed.

The single `cargo clippy -p aa-ui --lib --test chat_drag --example
native_chat_drag` completes successfully (exit0), retaining 27 pre-existing
library warnings plus the existing aa-dev dependency and aa-client manifest
warnings. It also identified duplicate inclusion of the original example's
fake module in the new harness; the harness now imports that existing fixture
instead, with only its example-local visibility widened. A final example-only
cargo check verifies the fixture cleanup; no second Clippy run or automatic
fix was used. The retained lint log is `chat-drag-clippy-20261023.log`.
Owned diff-check passes. Copied QA executables, saved-source scratch and this
task's Python bytecode cache have been removed; evidence/logs/PNGs are retained.

## Native SetTabButtonAlpha storage and inherited tab drawing (2026-10-23)

This closes the stub named in the preceding drag checkpoint. Original 1.2
registration `3925af17..68` names its parameters **selectedAlpha,
unselectedAlpha** and binds `39259fa0 ->3922f380 ->39258d90 ->392c8730`.
Both Lua parameters must be numbers. Missing/nil, strings (including numeric
strings), booleans and tables reject the entire pair; extra arguments are
ignored. The successful setter copies two raw f32 values to chat +358/+35c.
Constructor `392cb9d5/e5` initializes **1.0/0.3**, and duplicate configuration
`392cb87c..88e` copies them before the original creation callback. There is no
clamp, layout, selection callback, owner-layer update or scheduled fade.

The pair is stored but visually inactive in this client version. Complete
chat-widget/script-chat body inspection finds only constructor/setter writes
and the configuration-copy reads. Both native chat vtables inherit draw prep
`3927b420` and owner-layer drawing `3927af50`; script chat's frame is
`395130e0`. Executed hover/capture, unclipped/clipped preparation/render and
drag prepare/restore controls produce identical draw calls and restored render
alpha for 1.0/0.3, 0.05/0.95 and 1.5/-0.25. Actual manager `39502100/395021b0`
and global-pair configuration `39512a40` run too: hidden-input drag preparation
freezes OVER, and restoration enables dynamics then forces the completing
owner OVER and others LEAVE. None reads the tab-alpha pair or changes the
button/drawable alpha fields. Existing generic widget fading remains separate.

`ChatData` now holds a private pair, its Lua method follows the strict native
marshaller, and duplication copies it before OnCreateChatWindow. No public
Rust API, app hook, dependency, manifest or aa-client change is required.
The directly related old per-frame opacity estimate is removed: it forcibly
hid every tab drawable while idle and rewrote selected/unselected/add-button
opacity to 1.0/0.6. It had no matching instruction path. Original script
artwork, text colors, global dynamic drawable layers, explicit SetAlpha calls,
selection and drag preparation/restore now compose without that extra write.
This supersedes the older estimated tab-decoration behavior/gap descriptions,
including the alpha notes around the earlier native chat-options milestone.

`tools/native_chat_tab_alpha_oracle.py` retains **25 executed native groups**
and an instruction/vtable/property-access listing. FPCW is 037f. Read-only
x2game SHA-256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`;
CryScriptSystem remains
`75ad3b0e7b483b12a113fd6cb28bcab785259f28bca145cb3f28e0fa8e72f162`.
Lua VM primitives, host casts, pointer/capture geometry, font submission,
device color/clip submission and unrelated child frame primitives are explicit
endpoints. Native registration, typed conversion, property stores/copy,
script-chat frame, inherited draw traversal, dynamic layer propagation and
render-alpha push/restore are executed. This is not a full native font/GPU
renderer or undefined iterator-behavior claim. The JSON and source listing
remain in ignored `data/extract/aa-ui/native-chat-tab-alpha-*-20261023.*`.

The before regression records idle input replacing explicit alpha values with
`[1,0,.6,0,.6]`. New focused tests cover typed raw storage/defaults/zero-result
arity, configuration copy before reentrant creation overrides, retirement,
explicit alpha across physical hover/time changes, rendered layer factors,
visually inactive pair changes, original World/chat bytecode and physical
split/release with retained button/message identity and scrollback. The
existing bound and chat drag suites continue to protect synchronous callbacks,
caret/sliding/reorder, app-hook contracts and transient-state cleanup.

The aa-dev example `native_chat_tab_alpha` uses unchanged original World/chat
bytecode and artwork with two tabs, the same idle physical pointer and
SetTabButtonAlpha(.15,.85) in both captures. Matched 1920x1080 PNGs remain at
`shots/aa-ui/native-chat-tab-alpha-before-20261023.png` and
`shots/aa-ui/native-chat-tab-alpha-after-20261023.png`. Both were inspected
against each other and `reference/ingame/spawn/real.png`: AFTER retains the
authored selected/unselected tab backgrounds and original text colors, which
the estimated path suppressed/dimmed in BEFORE. The reference confirms the
idle art remains present; it is not a reference capture of this custom two-tab
fixture or a native SetTabButtonAlpha variation. Instruction execution supplies
the property-invariance proof. Both examples report zero script errors and
close through --shot after eight seconds.

FPS samples: BEFORE **386.4 / 409.6 / 441.3 / 395.3** (2.63 / 2.47 / 2.30 /
2.56 ms); AFTER **358.3 / 442.4 / 419.5 / 431.1** (2.81 / 2.28 / 2.42 /
2.35 ms). These are shared-machine capture observations, not an isolated
performance comparison. Build/capture logs remain in ignored
`data/extract/aa-ui/chat-tab-alpha-{before,after}-*-20261023.log`.

Final validation: **285 passing Rust checks across 32 aa-ui targets**, with
only the existing optional font-metrics test ignored. These are all 30 targets
from the preceding drag checkpoint plus chat_tab_alpha (4) and button_fading
(8); lib includes the two new private-storage/copy checks (31 passing total).
The complete exit0 run is `chat-tab-alpha-verified-20261023.log`. The new native
oracle's 25 groups, existing drag oracle's 32 and bounds oracle's 48 also pass;
their three `chat-tab-alpha-*-native-20261023.log` /
`chat-tab-alpha-native-20261023.log` logs are retained alongside the JSON/source.
Initial test runs caught fixture issues rather than implementation defects:
ColorDrawable does not expose SetAlpha (its common object factor is now
explicit host fixture data); pending hide retirement completes at the existing
frame boundary; a split window follows the release pointer and remains OVER
until a physical leave. Corrected controls test those existing contracts.

The single final `cargo clippy -p aa-ui --lib --test chat_tab_alpha --test
chat_drag --example native_chat_tab_alpha` succeeds (exit0), retaining the
same 27 pre-existing library warnings and existing aa-dev dependency/aa-client
manifest warnings. No new test/example warning, automatic fix or second lint
run is involved. Log: `chat-tab-alpha-clippy-20261023.log`. Owned diff-check
passes. All Cargo operations select aa-ui and the shared target; build-lock
waits were honored. Copied QA executables were removed; evidence/logs/PNGs
remain. No foreign path, original client/configuration/assets, server,
dependency or app integration was changed, and no push was performed.

This proves the bounded stored property and inherited drawing path. It does
not add a selected/unselected fade, a modern opacity policy, a native font/GPU
renderer, broader hover routing, or full chat/client parity. The prior drag
preview/release, retirement/reentrancy and synchronous bound behavior is
preserved without a new application contract.

### Query-driven slot tooltip providers (2026-10-23)

The existing `AAUI.SlotContent`, `SkillContent` and `ItemContent` tables may now
carry either `tooltip = existingTable` or `tooltip = function() return tableOrNil
end`. Only `slot:GetTooltip()` invokes a function, with **zero arguments**. The
existing table path keeps its identity and the live empty-slot path still
returns one nil. A provider's first result must be table/nil; no results become
nil, extra results are discarded, invalid result types raise a conversion error,
and provider errors propagate through the native method to its Lua caller. The
result is not stored back into the content table. Callable tables remain tables.

`GetTooltip` resolves its stored Lua value before calling the provider. No UI,
tree, registry app-data or slot-state borrow survives the call. A provider can
reenter the getter, change widget geometry, rebind the same slot, or retire its
owner. Its current table/nil/error result is preserved; subsequent calls on a
retired object retain the existing zero-result method behavior. Binding, content
copy, refresh, animation inheritance and frame updates never call the provider.
Original X2 method names/signatures and the Rust runtime API are unchanged.

The application-owned integration is an item-content closure such as
`function() return HUD.item_tip(s.id, s.grade, s.count, s) end`, carrying the
same item instance/detail8 data. There is no new arrival event or forced hover.
This patch changes only the toolkit getter; the application owns its producer
and score cache.

Read-only original evidence remains in ignored `data/extract/aa-ui/`:
`research_score_tooltip_native.py` and `research-score-tooltip-native.json`
execute four controls through original `392565b0` and `392c3c60`. Two calls on
the same bound item slot each reach the `39423560` tooltip builder. VM primitives,
host/accessor resolution and the builder itself are explicit endpoints. The
existing `music10.c`/`tip1.c` builder calls `39082550(id, true)` for a complete
stored-score tooltip; `391e1640 -> 39082260` saves/removes pending/copies notes
without an item-tooltip refresh in that body. LevelDB internals are not emulated.

`research_score_tooltip_lua.py` and `research-score-tooltip-lua.json` execute
original Lua 5.1 bytecode, retaining instruction streams while widening only the
chunk layout/constants. Original `inventory/sort_inventory.alb` hover queries
the adapter again; hovered `Update()` runs leave/enter again. Original
`baselib/tooltip/tooltip_view.alb` executes 100 item-tooltip frames with zero
additional item queries. Availability changes alone leave the open snapshot
unchanged; rehover obtains the title. Original score-section bytecode also emits
the two requirement/title rows with controlled text/native-provider inputs.

The related original bag-mask contract was checked separately, without changing
the application. Both `UpdateIcon()` and `OnEnter()` omit `needInfo` in the
original sort-inventory bytecode; `sort_bag.alb`'s adapter omits a nil argument.
Native `3944d6a0` defaults an omitted third argument to numeric mask -1. An
explicit third argument starts at zero and uses typed numeric reader
`39058d10 -> 3550a460/3550bf90`: explicit nil/false/true leave zero; numeric masks
are retained. Only exact -1 enters `39423560`'s full instance branch containing
the user-music query. Thus ordinary bag icon updates can prefetch a score, while
an explicitly restricted mask must not acquire the full score extras. Eight
executed wrapper/conversion cases are retained in
`research_score_tooltip_flags.py` / `research-score-tooltip-bag-flags.json`.

`tests/slot_tooltip.rs` adds six meaningful controls: existing table/callable
table/nil identity and arity; all three binding sources plus copy/refresh/100
frames; provider result/error/recovery; recursive query and same-slot rebinding;
table/nil/error during immediate owner retirement; and physical hit-tested
original action-slot entry. The latter runs unchanged World/action-slot/tooltip
bytecode, keeps the displayed lines and query count unchanged for 100 frames
after metadata availability, then leaves/reenters and observes the original
Harbor title row. It does not call the hover handler directly.

The aa-dev example `native_slot_tooltip` uses that same original action slot and
the database-backed item 28051 icon `icon_item_1260.dds`. Its controlled provider
becomes available at 2.5s, the pointer leaves/reenters at 4.0/4.3s, and the capture
exits unattended. `--snapshot` selects the existing eager-table producer as the
BEFORE comparison; this is a table-versus-provider comparison in one binary,
not a pre-patch executable. All three 1920x1080 PNGs were inspected:

- `shots/aa-ui/slot-tooltip-snapshot-before-20261023.png`: one bind-time query,
  no score title after rehover.
- `shots/aa-ui/slot-tooltip-provider-stationary-20261023.png`: provider available
  but the open tooltip still has no title.
- `shots/aa-ui/slot-tooltip-provider-after-20261023.png`: two hover queries and
  the original Artistry requirement / Title: Harbor rows after rehover.

The item tooltip's art, header and text hierarchy were compared with
`reference/ingame/tooltip_item/real.png`; that reference is a Teleport Book, not
this controlled score fixture. This example supplies metadata availability,
not a network/cache roundtrip. Parent-owned application fixtures cover that
integration. All captures report zero script errors. FPS observations: BEFORE
425.6/425.7/448.2/407.4; AFTER 344.2/292.9/420.4/369.8; stationary 408.3/432.4.
These are shared-machine observations, not an isolated performance comparison.
Capture/build logs remain at `data/extract/aa-ui/slot-tooltip-*-20261023.log`.

Final validation: **100 passing checks across 10 selected aa-ui targets**, with
only the existing optional font test ignored: lib 31, bound_changed 10,
changed_anchor 12, chat_drag 10, chat_tab_alpha 4, slot_completion 7,
slot_native_cooldown 6, slot_state 1, slot_tooltip 6, window_bounds 13. Log:
`slot-tooltip-tests-20261023.log`. The single final
`cargo clippy -p aa-ui --lib --test slot_tooltip --example native_slot_tooltip`
passes with the existing 27 library warnings and no new test/example warning;
log `slot-tooltip-clippy-20261023.log`. No alpha/drag/bounds path, manifest,
foreign crate, original asset, server or process was edited. The copied QA
executable was removed after capture; retained evidence and PNGs remain.


## Widget-owned modelview command/image bridge (2026-10-24)

The bounded local-shop bridge is implemented. It contains no actor assets, camera,
render target allocator, equipment/appearance service or renderer pass. The prior
`HOST_API_PROPOSAL.rs.txt` field/variant shapes are present without a signature
correction: opaque `ModelViewId`; `ModelViewCommand`/`ModelViewOp`;
`ModelViewSurface`; `ModelViewImage { image: Handle<Image>, extent_px: [u32;2] }`;
`ModelViewWork`; `UiRuntime::take_modelview_work(&self) -> ModelViewWork`;
`UiRuntime::set_modelview_image(&self, ModelViewId, Option<ModelViewImage>) -> bool`;
`UiModelViews`; and `render::Tex::ModelView(ModelViewId)`. No dependency, manifest,
lockfile or other existing public method signature changes are needed.

Register host systems with `.in_set(aa_ui::UiModelViews)`. The existing `UiFrame`
encloses pointer dispatch (including complete release callbacks and guards), Lua
frame callbacks, this host set, painting, held cursor and IME refresh. `UiInput`
still precedes it. App-data/tree borrows end before the host or Lua can reenter.
The private frame split retains the existing release callback order; its regression
fixture now registers the actual two systems and the production ordering sets.
Frame CPU statistics sum toolkit callbacks and painting; root host work is separate.

Drain once per host frame and process commands in vector order. Calls across
widgets share that ordering; no Init/Clear/animation/equipment command is merged.
Every native modelview incarnation has a process-unique monotonic opaque ID.
Exhaustion fails instead of wrapping. Clear keeps that ID, source crop, image
extent, texture request and accumulated rotation, removes the presented image and
internal displayed drawable, and queues ClearModel. Init reestablishes displayed
geometry without manufacturing an image. A valid host image is still required.
Retirement removes the association and queues one Destroy. Retired Lua receivers
retain their ordinary zero-result guard. Old/foreign tokens reject image and None
associations; old queued quads without a current association cannot create a batch.

The live-surface snapshot includes hidden widgets and reports inherited
visibility/fade `paint_admitted`, independently of renderer/model readiness. Root
must also remove IDs absent from its next snapshot, reject stale async job revisions
within one ID, and explicitly tear down targets/jobs on runtime replacement/stage
exit. There is no runtime-drop callback. A private runtime identity invalidates
coincidentally equal UI paint generations and removes old modelview cache entries.
Host-owned Image assets are never deleted by the toolkit.

Original native integer semantics are kept separate: default request 128x128,
model extent 64x64 and crop(0,0,128,128); nonpositive texture requests do not mutate
or queue; model extents retain signed integers; coords do not mutate before a
shown-image drawable exists. SetModelViewExtent creates its internal background
image at that point in ordinary drawable creation order, with the recovered
corner0/3 anchors. The shop's explicit extent 270x450 then crop(121,31,270,450)
is preserved. Clear removes the drawable; a later Init/extent restores it. Pixel
crop values may be negative or extend past the allocation and are not recropped.
Source integer arguments become f32 for the image UV path. Fractional numeric
arguments truncate at the integer boundary; complete original wrong-type/arity
marshaller parity remains outside the instruction evidence.

The toolkit does not clamp against a guessed GPU cap, widget size or window size.
Root applies the recovered sequential f32 aspect-preserving/truncating clamp using
its authoritative renderer screen dimensions and returns the actual allocation
extent. UV normalization uses that returned extent, not the request. Zero supplied
extents reject the association. Missing/withdrawn images produce no image quad.
Only `modelview` receives the supported local-shop methods: Init, texture/extent/
coords, shop mode, additive camera adjustment, EquipItem(u32), reset, animation,
clear and degree-based rotation setters/getter. Unsupported customizer/costume/
FOV methods retain their existing stubs and do not produce host commands.

AdjustCameraPos emits ordered f32 deltas; root must add them in the native camera
frame, not interpret them as an absolute pose. SetRotation/AddRotation convert Lua
degrees to f32 radians and queue the accumulated unwrapped radians. Clear/Init do
not reset rotation. GetRotation uses the recovered f32 inverse multiplier. Host
false-shop-mode is the original no-op; the queue retains the call for root to
interpret. EquipItem has no added grade argument. The original shop ALB ignores
Init's result. The native valid wrapper submits true then false, with its actual
VM-stack result unresolved; the toolkit conservatively returns one false rather
than inferring host readiness. This is an explicit remaining return-stack anomaly,
not an exact native readiness claim or an invented successful Init.

External images use the same drawable traversal, inherited alpha/fade, owner
layer factor, widget/global scaling, transformed ancestor clipping, child paint
order and closing overlay pass as ordinary images. They bypass pak-path texture
resolution. Image association or extent changes invalidate paint; unchanged
associations and pixel-only host Image changes do not. The cache keeps the actual
Image handle, updates even hidden cached materials on handle replacement, rebinds reused
batches, and removes image/material cache references on Clear/retirement/reload.
Replacing a handle with identical geometry does not upload the mesh again.

Root migration: the old salon.lua class-wide overrides suppressed these natives,
including ordinary PlayAnimation. Root reports its separate migration now forwards
original methods/returns for non-salon widgets and selects salon widgets through
original Initialize -> InitBeautyShop ordering. That source is root-owned and was
not edited or compiled by this bridge task. Root must run its scoped salon/full
initialization regression after the bridge source-freeze notice. InitBeautyShop
remains outside this toolkit release; root owns its salon implementation.

Read-only prerequisites were read completely: modelview native/API contract,
camera prerequisite and OBJECT_SERVICE_REPORT. Original object import is now closed
for 4 actors + 32 cameras; the older camera report's unresolved-import statement is
superseded by OBJECT_SERVICE_REPORT. Constructed-preview camera height/runtime
state authority remains separate. This bridge does not release aa-cry production
rendering, claim native scene/pixel parity, or supply missing root camera/actor/item
inputs. The concurrent camera-height research directory is untouched.


Validation on the completed bridge: `cargo test -p aa-ui --lib --tests` passes
**620 checks across 88 targets**, with only the existing optional font-metrics
check ignored. This includes all 612 preceding release/UI checks and 8 new bridge
checks. `verified-tests.log` is the final-source exit0 run, including the last
hidden-image cache correction. The earlier full 620-check passes are retained as
intermediate logs, not additional distinct test cases. The original ALB tests
load selected unchanged instruction prototypes through aa-pak/aa-lua, binding
controlled enclosing upvalues. They execute original init/camera-offset, equip,
reset, hide, held rotation and reset-button bodies without starting a commercial
window or invoking a purchase/network provider. Other controls cover raw integer
extent/crop behavior, native rotation bits/arity, class scope, cross-widget call
order, Clear/retirement/runtime identity, transformed clipping/alpha/child order,
cache replacement/cleanup (including hidden replacement), stale queued quads,
host-seam release ordering/reentry, and repeated pixel updates without uploads.

The auto-discovered aa-dev example `modelview_bridge` supplies two real 512x512
host Image assets containing a diagnostic grid. It includes ordinary child/overlay
composition, inherited parent clipping/.85 scale/.65 alpha, resizing at 1 s, hiding
at 2 s, handle replacement at 3 s, retirement at 4 s and a new incarnation at 5 s. It uses
`aa_dev::default_plugins` and `DevPlugin`; it supplies no actor or commerce data.
Both final 1920x1080 PNGs were inspected:
`shots/aa-ui/modelview-bridge-before-20261024.png` (0.8s) and
`shots/aa-ui/modelview-bridge-after-20261024.png` (8s). The latter run reports five
live surfaces/four admitted; the old ID rejects association after Destroy. From 6 s
through capture, pixel changes retain **8 rebuilds/20 mesh uploads**. Final-run
FPS samples are BEFORE 536.8 (1.87ms), AFTER 420.9/476.0/413.6/412.9
(2.38/2.11/2.45/2.43ms). These are shared-machine observations, not an isolated
benchmark. No matching shop/modelview/salon/beauty real-client reference capture
was found in the bounded reference filename inventory; these images validate the
diagnostic compositor, not a native character scene or pixel-parity comparison.

The single `cargo clippy -p aa-ui --lib --test modelview --example modelview_bridge`
exits 0 with 27 existing library warnings and no new-file diagnostic. It ran before
the final hidden-material correction; that correction subsequently passed the
full 620-check run and example rebuild/captures. No second Clippy run or baseline
cleanup was performed. The owned diff and new-file rustfmt checks pass. No Cargo
workspace run, alternate target, clean, dependency/manifest/lockfile change,
foreign-process termination, live client/server/network/commerce run or push
occurred. The copied QA executable was removed; no window is left running.

Small evidence remains under ignored `data/extract/aa-ui/modelview-bridge-20261024/`:
`api-present.json`, final build/test/capture/lint logs, source/artifact hashes,
`tex-literal-audit.*`, `tex-match-audit.txt`, accepted/newer-research baselines and
`preservation-check.json`. All 209 pinned prior files (a superset of the requested
accepted 200) remain byte-identical. Newer camera-height research was snapshotted
separately and not edited; the earlier native oracles were not rerun. The literal
and match audit found only the known aa-client cooldown Atlas comparison outside
aa-ui and no external exhaustive match of this Tex type or render-state literal.
Root's later coordination reports height-producer closure 47 cases and additional
original clamp rounding controls; those are root/other-owner evidence, not added
toolkit test counts or renderer release authority.

## ListCtrl manual-cell visibility correction (2026-10-24)

Original guard-tower rows allocate six `LCCIT_WINDOW` cells, create their own
labels/buttons/drawables, and fill them without `InsertData`. The toolkit hid
every new sub-item, suppressing all of those descendants while row separators
and the page's maximum label still rendered. Nonempty text getters did not
establish paint visibility.

Pinned native `x2game_unpacked.dll` SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`
shows each row and cell in `InsertRows` (`392a4f70`): row Show at `392a5026`,
unconditional cell Show at `392a5101`, for all three column types. The second
argument's registered name is `withEventWindow`; its branch starts at
`392a5127`, after cell Show. It controls the separate event window.
The final geometry callback does not refresh data visibility. Data mutations
separately call `392a42b0`, showing occupied cells and hiding empty ones across
the list. An unknown valid DeleteData key also reaches this refresh. Widget
Show and drawable SetVisible do not implicitly show a hidden owner.

The approved correction removes only the initial cell hide and performs one
visibility sweep per data mutation. Manual rows keep `has_data=false` and no
key. Existing key allocation/update/free-slot behavior remains unchanged.
DeleteAllDatas clears flags before a single sweep; invalid DeleteDataByIndex
does not refresh. There is no geometry, picking, renderer or public API change,
territory-specific override, dummy data, or parent force-show.

`tests/listctrl_visibility.rs` adds seven actual UiRuntime regressions. They
cover type 0/1/2 and mixed columns, six initially visible manual rows, effective
paint traversal and generated label/drawable quads, hidden-cell/ancestor
negatives, data sweeps, append, unknown delete, empty DeleteAllDatas, valid and
invalid index deletion, resize/header/gap controls, and existing key holes.
The manual descendant button is picked, receives OnEnter and OnClick, and
leaves row selection/hover index at -1 with every occupancy flag false.
Quad assertions account for physical UI scaling. These checks do not start a
GPU or client/server connection.

The exact approved proposal and research remain frozen under ignored
`data/extract/aa-ui/manual-listctrl-visibility-20261024/`: **28 bounded native
cases and two complete original-ALB cases**, with explicit endpoints. Release
logs, source hashes and preservation checks are separate under
`data/extract/aa-ui/manual-listctrl-visibility-release-20261024/`.

No-column InsertRows allocation, `withEventWindow=true` creation, and native
DeleteData data-vector compaction remain unresolved broader-goal gaps. This
hotfix preserves the approved raw shown-state refresh rather than expanding
into native Show fade/callback lifetime parity. In particular, a data refresh
after an explicit cell hide is not a new fade-restoration contract. The
negative ancestor tests use actual Show/SetVisible and verify the established
subtree gate independently.

Root owns the post-fix five-capture matrix. The original v1 PNGs/logs and copied
120126976-byte fixture executable remain immutable; that executable's SHA256 is
`9405fd54b37bb3505ce64261acafa7901e507f20f6447439dafa93ca2d421833`.
The retained guard capture is still **visual FAIL** until Root's rerun proves
the six badges/names/buff icons and current-step highlight. Independent retail
origin/NPC-query integration also remains held; this correction does not
release the full territory feature or claim retail pixel parity.

Final-source validation: `cargo test -p aa-ui --locked --lib --tests` passes
**627 checks across 89 targets**, with the existing optional font-metrics check
ignored; `cargo test -p aa-ui --locked --doc` passes both doctests. The focused
run passes 7 new visibility, 6 unchanged key and 4 unchanged geometry cases
(included in the full count). One final
`cargo clippy -p aa-ui --locked --lib --test listctrl_visibility` exits 0 with
27 existing library warnings and no diagnostic in the new test or changed
production logic. The existing aa-dev dependency warning was left untouched.
No source correction followed these final-source tests or Clippy. New-test
rustfmt and the owned diff whitespace checks pass. All 43 pinned prior research
and v1 capture artifacts remain byte-identical. No GPU/FPS run, foreign-crate
Cargo command, manifest/lock change, server/network operation or push occurred.

## ListCtrl mouse double-click option (2026-10-25)

`SetUseDoubleClick` now stores the per-list flag instead of being a no-op. Native
initialization `392a383f` sets it false; the boolean binding adapter `39247010`
writes it at list +0x310. Mouse dispatch `392a3e00` and ordinary selection
`392a3a20` establish three distinct cases:

- With the flag false, every left click follows ordinary selection, reporting
  `OnSelChanged(index,false)`. Clicking the selected row again clears selection
  and reports index 0, whether the click is rapid or slow.
- With the flag true, an ordinary click selects its row, including an ordinary
  repeat on the selected row; its notification still reports false.
- With the flag true and an admitted double-click, the callback reports the
  **existing** selected index and true. It does not reselect the hovered row or
  change/show/reanchor the selection image. An empty selection reports 0,true.

The mouse path reads the option and selection after script OnClick delivery,
without holding a toolkit borrow across callbacks. Retired/disabled lists do not
gain a selection. Scripted `Select` and the public `widget_set_selected` helper
retain their existing repeated-fire/index/suppression behavior. Data keys,
occupancy, manual-cell visibility and data-refresh sweeps are unchanged.
The only Rust data addition is `ListCtrlData::use_double_click: bool`, default
false; no application wiring or new host API is required.

This is a bounded mouse-entry correction. The existing mouse classifier's
target identity and 350ms timing rule are unchanged and are not claimed to be
native timing parity. The native ordinary-selector requirement for a selection
image remains outside this patch: toolkit callers still select without one, as
before. Complete wrong-type/arity marshalling, native sounds/layer effects and
programmatic Select parity are not claimed.

The earlier preferred event-window gap was investigated first. The bounded
existing-decomp caller inventory has no direct application `InsertRows(...,true)`
call; page/scroll wrappers accept and forward a variable flag. Native creation
executes inside the column loop and crosses a duplicate-name widget-factory
boundary that has not been fully resolved. It remains an explicit gap, along
with no-column row allocation and native DeleteData vector compaction. The
approved scope instead has thirteen direct original application calls to
`SetUseDoubleClick(true)`, including world-select, mail, auction, dungeon-channel
selection, ranking, friends and loot.

Research is retained separately under ignored
`data/extract/aa-ui/listctrl-events-20261025/`. The original pinned DLL SHA256
remains `851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
`native_doubleclick.py` executes **47 bounded native cases** through the actual
initializer, boolean setter, click and selection instruction streams. Geometry,
factory, sound and callback delivery are named controlled endpoints; no DLL or
game process is started. The matrix covers both options, all selected/hover
combinations for none/row0/row1, both supplied double flags, nonleft buttons,
and the native selection-image guard boundary. Three original wrapper/dungeon
ALBs were freshly compared with aa-pak bytes; their file and selected instruction
hashes are in `original-consumer-prototypes.json`.

`tests/listctrl_doubleclick.rs` adds seven actual UiRuntime CPU regressions.
They drive real picking, press/release, callbacks, selection-image state and
negative input admission. Unchanged original page and scroll wrapper setter /
OnSelChanged prototypes feed the original dungeon-channel SelChangedProc; a
controlled local action counter verifies that a true notification activates
once, while disabling the option clears selection and disables that action.
This exercises original ALB instructions through the actual toolkit, without a
game API provider or network action. It is not a visual deliverable or a claim
of native pixel parity; no GPU slot or FPS capture is needed for this scope.

Final-source `cargo test -p aa-ui --locked --lib --tests` passes **634 checks
across 90 targets**, with the existing optional font-metrics check ignored.
The focused 24-check ListCtrl run (7 new, 6 keys, 4 geometry, 7 visibility) is
included in that count. All 30 files in the earlier manual-cell native evidence
directory remain byte-identical. This slice changes no manifest, dependency or
lockfile, and uses no alternate target, workspace test, GPU, live client/server,
foreign-process termination, mount implementation or push.
Both doctests pass. The single final
`cargo clippy -p aa-ui --locked --lib --test listctrl_doubleclick` exits 0 with
27 existing library warnings and no diagnostic in the new test or changed
production logic. No implementation/test correction followed the final focused,
full-suite, doctest or Clippy runs. New-test rustfmt and owned whitespace checks
pass. The shared Cargo target was used throughout; root resolved its separate
aa-client lockfile entry before the locked validation runs.

## ListCtrl EnableScroll clipping (2026-10-25)

The ListCtrl class no longer replaces the inherited `EnableScroll` method with
a no-op. The unchanged original shared scroll-list constructor calls
`listCtrl:EnableScroll(true)`; the achievement objective constructor explicitly
calls false. These now control actual paint and pointer admission.

Native registration `3926a050` binds `EnableScroll` to adapter `39265820`, which
writes the base widget's `+a4` marker. ListCtrl has no separate setter or header
clip area. Lookup `39277a10` starts at the widget itself and returns the full
physical rectangle via `392754a0`; changing column height does not subtract
anything from this area. The inherited renderer `3927b010` pushes this rectangle
before delivering the list's own ordinary layers and content, then delivers
descendants under clipping. Thus enabling only descendant clipping would still
let list-owned background/artwork/overlay and selected/hover images overflow.

aa-ui reuses `Widget.clip` and adds no ListCtrl data or public host API. Its clip
lookup includes an enabled ListCtrl itself, while ordinary scroll widgets keep
their established descendant-only behavior. The full list rectangle clips the
header, rows, manual cell descendants and owned drawables. False removes only
that list's boundary; clipping ancestors still apply. Geometry, scrolling,
manual shown flags, occupancy, key allocation, scripted Select and double-click
dispatch are unchanged. No territory-specific visibility or clipping rule and
no event-window behavior were added.

Native inherited picker `3927c6d0` takes both inside-parent and outside-parent
child traversal branches. Its clip comparisons admit exact left/top/right/bottom
edges. ListCtrl clipping now preserves that inclusive boundary; a descendant's
own ordinary hit rectangle remains half-open, as do other scroll widgets'
existing clip bounds. A clipping ancestor can therefore still reject a point
on its ordinary excluded edge even when a nested list admits that point.

New ignored evidence is isolated under
`data/extract/aa-ui/listctrl-clipping-20261025/`. The pinned DLL SHA256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
`native_clipping.py` executes **25 bounded native cases**: four setter state
combinations, six complete-rectangle/render cases with header heights 0/30/70,
fourteen pointer cases spanning both traversal branches and exact edges, and
disabled-list inheritance. Marker lookup, physical rectangle arithmetic,
native render/pointer traversal and clip push/pop execute original instructions.
Binding host casts, alpha return and layer/content submission are explicit
controlled endpoints; no DLL or original process is loaded. The two original
consumer ALBs were freshly compared with aa-pak bytes; their full-file and
selected unchanged prototype hashes are retained separately. All **517 pinned
earlier research artifacts** are checked for preservation.

`tests/listctrl_clipping.rs` adds **eight actual UiRuntime CPU regressions**.
They verify all four partially off-edge manual buttons' paint, enter/click
delivery and occupancy negatives; header-area descendants and an oversized
header; list-owned ordinary layers and selected/hover image UV cropping;
inclusive clip edges versus ordinary hit bounds; clipping ancestors, siblings,
hidden cells and hidden owners; and current scrolling/scale/resize boundaries.
The unchanged original scroll wrapper constructor, layout/data methods and
selection callback run through the real toolkit. The unchanged original local
achievement constructor is recovered from its actual closure chain, retaining
its original upvalues, and proves the explicit false overflow behavior.

This is a bounded ordinary-layer clipping correction, not native renderer or
pixel parity. The existing physical paint snapping remains. Native clipped
traversal omits overoverlay layer 3; aa-ui's earlier overoverlay convention is
preserved and that native difference remains open, as do escape/skip flags
`+26c/+26d`, full Lua wrong-type/arity marshalling, and the earlier ListCtrl
event-window/no-column/vector-compaction gaps. No GPU capture or FPS measurement
is needed for the tested paint geometry/UV and pointer contracts. No live
client/server, network operation, foreign process termination or mount work
occurred.

Final-source `cargo test -p aa-ui --locked --lib --tests` passes **642 checks
across 91 targets**, with the existing optional font-metrics check ignored;
both doctests pass. The eight new checks are included in that full count.
The single final `cargo clippy -p aa-ui --locked --lib --test listctrl_clipping`
exits 0 with the same 27 existing library warnings and no diagnostic in the new
test or changed logic. All commands use the shared target and select aa-ui only.
An independent read-only review found no actionable P1/P2/P3 and independently
verified the native source, original ALBs and all 517 preservation pins. The
reviewed implementation and tests remained unchanged through final validation.
New-test rustfmt and owned diff whitespace checks pass. No manifest, dependency
or Cargo.lock edit is part of this slice.

## ListCtrl clear-all selection ordering (2026-10-25)

`DeleteAllDatas` now clears selection and reports `OnSelChanged(0,false)` when
the list has a selected image. The notification also occurs for repeated or
empty clears. It runs before data destruction: a handler can query the old
keys/cells, append or replace rows, insert data, reselect, nest a bounded clear,
or retire the receiver. After delivery the method checks liveness and clears
the current rows. Data inserted by the handler is cleared; handler reselection
is retained. No toolkit borrow spans Lua delivery. Retired receivers skip the
remaining mutation under the established host liveness policy.

Native `392a5750` calls selector `392a3a20(-1,true)` at `392a5759`, before reading
the record cursor at `392a575e`. The selector requires the image at `+30c`, hides
it, stores selection -1, and delivers event 7 with index 0 and false. The toolkit
uses its existing internal `set_selected(-1,true,false)` and immediately flushes
the notification before reacquiring list state. This method keeps its native
image-present guard; the toolkit's no-image selection policy and standalone
`ClearSelection` repeated-call suppression are unchanged. Hover index/image,
stable keys and holes, capacity, one-based index deletion, clipping, double
click behavior and public host APIs are unchanged.

The real consumer is the original dungeon-channel refresh. Its unchanged
`FillData` (`indun_channel_select.alb main/f0/f9`, original offset `0x18a7`,
lines 176-201) calls the scroll wrapper's `DeleteAllDatas` without a separate
`ClearSelection`. Previously, after instance 701 was replaced by 909, the
wrapper retained key 701 and the new row remained highlighted. The native
clear callback resets the wrapper selection before refill. The original default
texture helper creates the selection image, so this path satisfies the guard.
The action button is explicitly disabled by FillData; the defect and regression
are selection/highlight state, with no dungeon-entry or network claim.

`tests/listctrl_clear.rs` adds nine actual CPU UiRuntime regressions. The consumer
test loads the unchanged world-stage ALBs, constructs the original dungeon
window and scroll wrapper, uses their original texture helper, FillData, data
setters and selection callbacks, and verifies replacement and empty refreshes.
Only dungeon-state/name/entry APIs are controlled locally; the entry endpoint
fails if called. The other tests cover pre-clear key/occupancy queries, repeated
and zero-row clears, hidden/disabled lists, no-image behavior, current appended
and replaced rows, insertion, reselection, nested clear, receiver retirement,
hover preservation and standalone suppression. Existing key, geometry,
visibility, double-click and clipping checks remain unedited.

Native compaction is a separate unresolved compatibility decision. Original
keys survive middle deletion, while later data records move into earlier fixed
row widgets; selection and hover indices do not follow the key. The toolkit's
approved hole/capacity/index contracts remain in force. Examined portal callers
immediately rebuild the page, and scroll wrappers delete native tail records;
this research did not establish a persistent compaction-only consumer failure.
The earlier direct-native classification of premium `benefit_view` was too
broad: its constructor obtains the Lua scroll wrapper.

Frozen research remains under ignored
`data/extract/aa-ui/listctrl-delete-20261025/`: 38 native cases, six original-ALB
cases, seven fresh Pak comparisons and the original 806-file preservation
snapshot. DLL SHA256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
New release logs and preservation/review records live separately under ignored
`data/extract/aa-ui/listctrl-delete-release-20261025/`. The native allocator,
reserve/free, CRT, host-cast, factory/geometry, text and script-dispatch endpoints
are explicit in the frozen report; neither those probes nor the UiRuntime tests
claim arbitrary native lifetime safety or GPU/pixel parity. No GPU/FPS capture,
live original client, game server, network operation or mount work is needed.

Locked shared-target validation passes **651 checks across 92 targets**, with
the existing optional font-metrics check ignored, plus both doctests. The
41-check focused run includes all nine new cases and the unchanged key,
geometry, visibility, double-click and clipping targets. New-test rustfmt and
owned diff whitespace checks pass. Preservation verifies all 543 inherited
research artifacts plus all 24 files of the frozen delete research directory
remain byte-identical; its original 806-file manifest/results are untouched.
No manifest, dependency or Cargo.lock edit is part of this slice.

Independent read-only ClearAll review is CLEAR with no actionable P1/P2/P3.
It verified the ready-source bytes, DLL/11 native spans, seven original ALBs,
567 preservation artifacts and validation logs. The single final
`cargo clippy -p aa-ui --locked --lib --test listctrl_clear` exits 0 with the
same 27 existing library warnings and no diagnostic in the changed logic or
new test. Reviewed production/test bytes remained unchanged through Clippy;
only this validation record was added afterward. No frozen evidence was rerun
or rewritten, and no production/test correction followed final validation.

## Native Window background picking (2026-10-25)

`window:EnableWindowPick(bool)` now controls native background admission instead
of logging a stub. A fresh Window takes a background hit without requiring a
script mouse handler. False suppresses that Window and descendant Window
backgrounds; an inner true cannot override an outer false. Buttons, edit boxes
and other non-Window descendants keep their existing input behavior. True
restores ordinary creation/Raise and UI-layer ordering. The flag is independent
of `EnablePick`/`Clickable`, shown state and scroll clipping.

Native registration `3926b850` binds the Window adapter `3926aa30`; its actual
boolean entry is `3926b350` -> `3926b2a0`. The entry accepts a present boolean
variant (type2) and returns zero results for rejected/missing arguments. Adapter
`3926aa30` obtains the Window receiver before calling `392d3c60`, which stores
the inverse flag at `+2e9`. Initializer `392d3e40` reaches `392d3fc3` for a new
Window without a template and zeroes both `+2e8` and `+2e9`; the earlier template
copy branch is separate. Picker `392d3b60` forces the inherited skip-Window
argument when `+2e9` is set. Base picker `3927c6d0` visits children before its
`3927c805` native-type0 self-return guard, so buttons remain eligible through
suppressed Window ancestors, including when they extend outside an ancestor.

aa-ui adds `Widget::window_pick`, default true, implements the Window-only
method, and checks Window ancestors when considering a Window pointer target.
Window backgrounds join native input admission in the existing picker. No
key/focus/event routing, renderer, geometry, ListCtrl method or host integration
changes are required. Existing `EnablePick` behavior remains unchanged; this
slice does not claim its separate native subtree-veto parity. The same-named
Grid binding writes a different `+405` flag. Its runtime consumer remains
unclosed, so Grid's existing stub is deliberately retained.

Original character-info `UpdateHud(show)` toggles the Window flag after its
original `ReleaseHandlers`; false exposes underlying input while retaining
equipment buttons, and true restores background capture even after the click
handler was released. Original login `login_view.alb` disables its full-screen
game-layer `backMain`, keeping lower non-Window targets reachable. The original
`SetViewOfEmptyWindow` constructor creates class `window`, so these consumers
exercise this class contract. The complete extracted original ALB inventory has
four files containing `EnableWindowPick`, including the separate Grid callers.

Ignored evidence is isolated under
`data/extract/aa-ui/grid-window-pick-20261025/`. The original DLL SHA256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
`native_window_pick.py` executes 25 bounded original-x86 cases covering new
initialization, binding/boolean entry, background/child/button combinations,
both inside/outside-parent traversal, overlapping siblings, false-to-true
restoration, hidden ancestors, inherited suppression and missing receivers.
Constructor manager/text/factory services, host casts and the variant argument
supplier are explicit controlled endpoints. Constructor execution stops at
`392d3fe8`, before unrelated child creation. Actual setter, boolean validation,
rectangle/clip arithmetic and Window/base pointer traversal execute native bytes;
no DLL is loaded or original process started. Five original ALBs match fresh
aa-pak reads, with full-file and selected unchanged instruction hashes retained.
All 2,173 prior research artifacts in the combined new/inherited preservation
snapshot remain byte-identical, including the earlier 567 frozen release pins.

`tests/window_pick.rs` adds seven actual CPU UiRuntime regressions for overlapping
and layered Windows, Raise, both booleans, ancestor false/descendant true through
a non-Window intermediary, retained buttons and edit focus/typing, hidden and
clipped/outside descendants, retirement, class/boolean scope and independent
EnablePick. The original character-info UpdateHud and ReleaseHandlers bodies run
unchanged with controlled equipment/tooltip endpoints; the full original login
script stage runs with the local fake API and an empty account-name provider.
No authentication, game service, GPU or live client is invoked. These checks
establish input/runtime behavior, without a pixel-parity or FPS claim.

Final-source `cargo test -p aa-ui --locked --lib --tests` passes **658 checks
across 93 targets**, with the existing optional font-metrics check ignored;
both doctests pass. All 651 preceding checks and the seven new cases are
included in that count. The single final
`cargo clippy -p aa-ui --locked --lib --test window_pick` exits 0 with the same
27 existing library warnings and no diagnostic in the new test or changed
production logic. No implementation/test correction followed the successful
focused, full-suite, doctest or Clippy runs. The frozen production/test hashes,
validation logs and preservation checks are retained beside the research.
New-test rustfmt and owned diff whitespace checks pass. No manifest, dependency,
Cargo.lock, foreign-crate source or ListCtrl test was edited. No workspace run,
alternate target, clean, GPU/FPS run, live endpoint, server restart, foreign
process termination, mount work or push occurred. Independent R1 review is
CLEAR with no actionable P1/P2/P3; root's source/test/docs review is also clear.
The reviewer verified all five ready-source pins, and root verified the four
frozen production/test pins. Root accepted the final validation and released
the five owned paths for commit in its explicitly granted index window.

## Native Window subtree picking and Shift roadmap input (2026-10-25)

`window:EnablePick(bool)` now controls its independent subtree gate instead of
sharing `Clickable` state. A disabled Window excludes its own background and
every descendant pointer candidate, including buttons outside its rectangle.
A descendant true cannot override it. `Clickable` still controls the receiver's
own admission; `EnableWindowPick` still suppresses only Window backgrounds
through the subtree. The three flags remain independent. Window EnablePick
accepts a present literal boolean and returns zero results; missing/nonboolean
arguments leave it unchanged. Other classes' EnablePick policy is unchanged.

Native registration `3926b821` -> adapter `3926a9f0` -> setter `392d3c40` writes
inverse enable at Window `+2e8`. The same strict boolean entry `3926b2a0` used
by the preceding WindowPick release supplies the value. Fresh no-template
Window initialization zeroes `+2e8/+2e9`. Window picker `392d3b60` tests `+2e8`
before inherited child traversal, whereas base `Clickable` setter `39277940`
writes `+bb`, checked after child traversal. aa-ui adds `Widget::pick_enabled`,
default true, and uses this Window-specific gate in the existing ancestor walk.
No data key, row geometry, selection, clipping or ListCtrl method changes.

The roadmap and radar-road tooltip checks previously read the top Window's
shared `clickable` flag. Native source proves a different authority: Roadmap
update `392c07e0` samples physical modifier mask **0x22**, calls shared map
tooltip builder `392b6f20` with event **0x199**, then publishes
`SET_ROADMAP_PICKABLE` (**0x19c**) only on a per-map state transition. Mask 0x22
is **Shift**: original `IsShiftKeyDown` binding `3947d420` uses it; Control
`3947d4b0` uses 0x11 and Alt `3947d540` uses 0x44. Bit test `394245a0` checks
their intersection. Tooltip-builder admission is independent of the parent's
three Window flags. A direct Window-flag migration would retain a false proxy.

Roadmap initializer `392c1730` explicitly clears its own `+688` cached state.
The update calls the tooltip builder before comparing/storing that state;
the cache is committed before delivering the transition callback. Release
resets a selected tooltip/controller before false delivery. aa-ui now uses its
existing physical left/right Shift state for both ordinary and radar roadmap
tooltips, retaining their existing visibility/geometry/content checks. The
WorldMap hover branch is unchanged. The unchanged original road_map.alb
event handler receives the boolean and calls `roadmapWindow:EnablePick`.
No script replacement, new Lua/host API or aa-client migration is needed.

The per-map cached Shift bit belongs to RoadMapData and starts false. Its pass
runs at the existing widget frame walk's **post-child derived-update slot**:
native `3927e286..e29f` invokes virtual `+158` after the live shown-child walk.
Thus existing root/fade admission, hidden-child exclusion and live liveness
checks determine whether it runs. This includes a shown child of an admitted
fading root; it does not add a global scan that steps hidden maps. Projection
and data publication retain their earlier scheduling. Only roadmap tooltip
work moves into the admitted slot, before the transition. The pass also works
without a host map-data hook, using retained state; no global default is inferred
from a Window flag. A map created while Shift is held gets its own first true
transition. Hidden maps retain their last admitted state until admitted again.

No toolkit borrow spans tooltip/transition callbacks. Cache-before-delivery
prevents duplicate events during a bounded reentrant tick. A callback can hide
or retire a map; the continuation rechecks host liveness before further access.
This retains the toolkit's retirement policy without claiming native freed-
pointer safety or new host-factory lifetime semantics.

`tests/window_enable_pick.rs` adds eight CPU UiRuntime regressions. The two
original World-stage cases were first run against unchanged production and
failed as expected: physical Shift produced neither the original transition
nor visible tooltip, and Clickable(true) undid the original false handler's
Window gate. Both now pass. The original normal and ship tooltip paths run
unchanged, including numeric radar identity and health metadata; both Shift
keys, Control/Alt negatives, held-key transition suppression, manual Window
flags and release hiding are checked. Other cases cover strict Window booleans,
an ordinary non-Window EnablePick control, overflowing buttons, new/hidden/
retired map identities without a data hook, post-child ordering, fading-root
versus hidden-child admission, reentrant delivery and retirement during the
original tooltip callback. No passive child was forced interactive to invent
a consumer failure. Original scripts use local fake/published game data only.
The first full-suite run exposed one older HUD fixture that used Window
EnablePick as tooltip authority. `tests/hud_widgets.rs` now proves that enabling
the Window alone produces no tooltip, then supplies physical Shift before its
retained marker-text/controller assertions. Its heading and geometry checks
are unchanged.

New ignored evidence stays under
`data/extract/aa-ui/window-enablepick-20261025/`. Four original ALBs match fresh
aa-pak bytes; selected unchanged instruction hashes and the two-failure RED log
are retained. DLL SHA256 remains
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
The new oracle passes **45 bounded original-x86 cases** for independent Window
flags/picking, strict boolean entry, the per-map initializer, Shift/Control/Alt
masks, parent-flag-independent tooltip-builder admission, transition ordering,
cache observation and release reset. Binding casts/variant supply, input-mask
supply, Map core/tooltip-builder delivery, script dispatch and cursor/anchor
services are explicit controlled endpoints. Flag writes, pointer traversal,
Shift test, cached transition and release-reset instructions execute unchanged.
No DLL is loaded and no previous oracle/report/capture is rerun or overwritten.
All **2,257** inherited research artifacts remain byte-identical.

This slice does not implement native Map controller factory/class parity,
arbitrary tooltip row/selection or frame scheduling parity, native cursor/drag
behavior, or non-Window EnablePick class semantics. The earlier ListCtrl
EventWindow/no-column/compaction and Grid +405 runtime-use gaps remain held.
No manifest, dependency, Cargo.lock or foreign source is changed. No GPU/FPS
run, live endpoint, server restart, mount implementation,
foreign process termination, alternate target, clean, workspace command or
child push is part of this work. Root owns the authorized whole-branch push
after this batch's testing and review.

Final CPU validation: `cargo test -p aa-ui --locked --lib --tests
--no-fail-fast` passed **666 tests across 94 targets**, zero failures, with
the one pre-existing `font_metrics` ignore. The corrected HUD target separately
passed all **6** tests; the focused five-target run passed **33** checks.
`cargo test -p aa-ui --locked --doc` passed **2** doctests. The single final
`cargo clippy -p aa-ui --locked --lib --test window_enable_pick` exited zero;
its **27** aa-ui warning sites exactly match the preceding accepted baseline,
with no added or removed sites. The new test's standalone rustfmt check and
the owned-path `git diff --check` passed.

Independent R1 found only the outdated HUD fixture's P2 tooltip assumption,
matching the full-suite finding; it found no additional production issue.
Fresh R2 is **CLEAR**, with no actionable P1/P2/P3. R2 verified all eight
production/test size/SHA256 pins in `production-test-freeze-v2.json`, including
the unchanged original seven pins and the corrected HUD fixture. The final
nine owned paths, including this documentation, are pinned in the new evidence
directory's `ready-source-pins.json`. Production/tests remain frozen after
review; this slice ends here, with no further feature scouting before root's
authorized checkpoint push.


## Native edit caret endpoint geometry (2026-10-25)

Original `baselib/edit_view.alb` initializes ordinary and multiline edits with
`SetCursorOffset(-2)` and `SetCursorHeight(2)`. These were incorrectly interpreted
as a horizontal displacement and a centered height extension. They now adjust
only the top and bottom caret endpoints, respectively. Both are raw physical
pixel deltas after UI/font sizing; neither changes the editable character boundary
on X. The existing literal-text baseline supplies the caret's baseline. Painting
uses native `top = baseline - fontHeight + 3 + offset` and
`bottom = baseline + 3 + height`, subtracts one pixel on both axes for text-style
snap, and submits endpoint pixels through f32 `+0.5` / signed truncation.

Single-edit registration `3923ab19/48` installs adapters `39239a30/70` through
`3923a480` -> integer thunk `3923a390` -> parser `3923a180`. Multiline
registration `3923bc24/7f` installs `3923af70/b0` through thunk `3923b680` ->
parser `3923b530`. The two parsers require the numeric variant (type4), first
narrow it to f32, and apply CVTTSS2SI. Missing/nil/bool/string/table inputs leave
the endpoints unchanged. Nonfinite/out-of-i32-range values become signed integer
indefinite (-2147483648), then the adapter converts that signed integer back to
f32. Fractional values truncate; even Lua integer2147483647 first becomes
f32 2147483648 and therefore becomes integer indefinite. There is no inferred
positive-height clamp. Wrong edit-class receiver casts return zero results.

The adapters store single-edit `+3e0/+3e4` via `392920f0/39292110` and
multiline `+3d8/+3dc` via `392945f0/39294610`. Actual single-edit painting at
`39293b8b..3bc6` and multiline painting at `392952af..52ea` both pass these
values to shared helper `39286500`. Its `392865a4..39286621` instructions leave X
independent of the parameters and construct the two vertical endpoints. Both
parameters and constant3 are added after font-height lookup. Style bit4 shifts
the rectangle by(-1,-1); the admitted ordinary draw's `392206f5..3922079d` performs
`+0.5`/CVTTSS2SI per endpoint. Native font-height reader `392e40d0` reads the
FT size metric at+20 with factor1/64; this oracle controls that metric rather
than inventing a live font instance.

New evidence is retained in ignored
`data/extract/aa-ui/active-grid-pick-20261025/`, a new scouting/caret directory.
`caret-native-cases-v4.json` contains **78 unchanged-original-x86 cases**:
shared helper6, four signed-integer adapters plus negative casts28, both integer
parsers16, shared helper through actual draw-rounding continuation4, and
actual Cry Lua parameter conversion24. Timer,
binding receiver lookup, zero-result delivery and final draw capture are
explicit endpoints. The24 full Lua-facing cases also execute original
CryGetParam3550a460 and requested-number conversion3550bf90; only Lua stack
top/type/number primitives and error reporting are supplied. Actual FLDqword
return followed by original FSTPf32 proves narrowing before integer conversion. Actual SingleEdit/MultiLineEdit vtables and their
positive/negative cast instructions execute unchanged. The helper, parsers, four adapters/setters,
native FT height reader and rounding instructions execute unchanged. No DLL is
loaded. DLL SHA256 is
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
The first46-case and both54-case reports remain separately retained. Cry DLL
SHA256 is `75ad3b0e7b483b12a113fd6cb28bcab785259f28bca145cb3f28e0fa8e72f162`. Fresh aa-pak extraction of
`edit_view.alb` matches retained bytes, SHA256
`6964ad5edb6ed0cb2b67d892318fc3bd4a7fd08190423ee92a6b999854c398bc`;
its InitEdit46 / InitMultiline48 unchanged instructions are independently pinned
in `original-edit-alb-pins.json`. The original RED log records three real
UiRuntime failures before production edits, including the original-script paint
failure. No established regression was edited to fit this contract.

This is an endpoint correction, not full native caret parity. The existing
530ms blink half-period is unchanged; the native shared helper's own timer policy
is independently outside this slice. The existing caret-width policy is
unchanged; the mutable native width global `39ac6e78` has not been given guessed
initialization authority. The original font/cap-height alignment services,
absolute native text-origin calibration, general inherited animation scaling
and OS/GPU input timing are not newly recovered here. The corrected caret
uses the existing literal-text painter's baseline and clipping/ancestor
composition. No public API, dependency, manifest or Cargo.lock change is needed.

CPU regressions in `tests/caret_geometry.rs` inspect actual emitted caret quads:
independent endpoint deltas at UI scales1/.85/1.5 for both edit classes; position
relative to the actual painted glyph baseline; native constant3 and style snap;
strict argument/receiver behavior and f32/signed-integer edge values; and actual
unchanged InitEdit/InitMultiline through base toc loading, focus, editing,
hiding and reopening at all three UI scales. `examples/native_edit_caret.rs`
uses aa-dev, the original edit initialization/skin assets, single and multiline
fields, and an explicitly deterministic visible caret phase for captures.
`--multiline` selects multiline focus; `--scale85` selects UI scale.85.
The diagnostic does not claim a matching native scene. The retained
`reference/character_create/step3_nuian_male/real_ability_name.png` was inspected;
it supplies no usable visible-caret geometry for this endpoint comparison.

The initial current-HUD CPU inventory is retained as `active-stubs.log`. Grid's
`EnableWindowPick`+405 runtime use remains unproved; the bounded displacement
search did not establish a read. Grid background drawing also requires native
row/column textures absent from its actual original quest-list caller. Neither
held boundary was changed. ModelView camera/FOV and bag-bound item-cooldown
provider work remain separate. Accepted Window picking/physical-Shift Roadmap and
ListCtrl stable-key/manual-visibility/scriptSelect contracts are preserved.


Both initial aa-dev captures were inspected and self-closed sequentially under
root's explicit serialized GPU grant:
`shots/aa-ui/native-edit-caret-single-1080-20261025.png` (UI scale1) and
`shots/aa-ui/native-edit-caret-multiline-720-20261025.png` (UI scale.85).
The caret is visible at an actual internal text boundary in each original skin.
Each reports zero script errors and no WARN/ERROR. FPS samples are
**391.2/373.0/354.9/341.8** (1080p) and **276.8/349.5/309.8/348.2** (720p).
These are shared-machine observations, not isolated performance measurements.
The fixture's visible-phase override is explicitly for deterministic captures;
it supplies no evidence for the native blink timer. The copied QA executable
was removed and the GPU slot explicitly released with no own process/window
left running. A subsequent R1 correction and final-source multiline capture
are documented below.


Independent CARET R1 found one P2 in the toolkit baseline producer: at UI
scale.85, innerY116, lineSpace2, no scroll and snap disabled, second-line caret
Y was scaled before rounding the origin. The glyph painter instead rounds its
physical origin first, then adds the physical line step. A new actual-paint
regression reproduced the error before any production correction: caret
endpoints `[114,127]` versus the second painted glyph's baseline requiring
`[115,128]`. `r1-multiline-red.log` retains that failure. The correction now
forms the baseline from rounded painter origin plus
`line * (physicalLineHeight + lineSpace * uiScale) + ascent`, grouped as the
glyph layout and painter group it. The original native endpoint arithmetic,
class casts, signed-integer parameters, width policy and blink qualification
are unchanged. This aligns to the toolkit's actual literal-text painter; it
does not claim a new native font/cap-height calibration.

Only `src/lua/edit.rs` and `tests/caret_geometry.rs` changed after the first
freeze. All five exact production/test/example size/SHA256 values are retained
in `production-test-freeze-v2.json`; the other three pins match v1. The full
v1 run passed670 checks across95 targets with the one existing font-metrics
ignore; that run is explicitly interim, preceding the R1 correction. The new
fifth focused test checks the actually emitted multiline glyph/caret geometry
at the reported fractional-spacing/scale counterexample. Root was asked to
commission fresh independent R2 against v2 before the coordinated index window.
No Clippy run had been queued or performed when the R1 correction landed.


The single corrected-v2 capture was rebuilt from final source and inspected:
`shots/aa-ui/native-edit-caret-multiline-720-r1-20261025.png`, UI scale.85,
selfclosed exit0, zero script errors/WARN/ERROR. FPS samples were
**421.5/328.9/327.7/343.7**, shared-machine observations only. The copied final
QA executable was removed and the GPU slot explicitly released; no own window
or process remains. The first two captures are retained as pre-R1 diagnostic
artifacts, not substituted for this final-source capture.

Read-only pixel comparison of the initial/final matching multiline captures
finds exactly2 changed pixels within `[532,380,533,398]`, the two caret endpoint
pixels (`r1-capture-pixel-diff.json`). This compares the diagnostic to itself;
it is not an original-client image-parity claim. The combined preservation
manifest verifies all **2,331** prior evidence artifacts byte-identical,
including the preceding accepted2,257 and this task's1,881-file starting
snapshot union. No previous native report/oracle/capture was overwritten.


Fresh independent CARET R2 is **CLEAR**, with no actionable findings. R2
verified all5 v2 bytecounts/SHA256 pins and that reversing only the baseline
correction plus new glyph regression reproduces v1, while the other3 pins
remain unchanged. It confirmed the `[114,127]` to `[115,128]` counterexample
is covered against the actual second painted glyph. Production/tests/example
remain frozen after R2; only this documentation/final proof record is updated.
The sole final Clippy invocation was queued against v2 after R2 clearance;
no earlier Clippy run was consumed by the initial version or R1 correction.


Final-source v2 validation: `cargo test -p aa-ui --locked --lib --tests
--no-fail-fast` exits0 with **671 passing checks across95 targets**, zero
failures, and the one pre-existing optional font-metrics ignore. This includes
all666 accepted checks plus the five caret regressions; the focused target
separately passes5/5. `cargo test -p aa-ui --locked --doc` passes **2** doctests.
The sole final `cargo clippy -p aa-ui --locked --lib --test caret_geometry
--example native_edit_caret` exits0, retaining27 existing library warnings
and adding one test-only `DoubleEndedIterator::last` advisory for the new
regression's two-glyph scan. The R2-reviewed v2 source is deliberately retained
unchanged; no Clippy autofix, source edit or second Clippy run follows it.
The new test/example standalone rustfmt check and owned-path diff-check pass.

Final logs and the machine-readable result are `runtime-suite-v2.log`,
`r1-focused-final.log`, `doc-tests-final-v2.log`, `clippy-final-v2.log`, and
`validation-summary-final-v2.json` in the new evidence directory. The five
reviewed code/test/example pins remain unchanged. `ready-source-pins.json`
contains the final **six** owned paths, including this completed note.
No public API, manifest, dependency, Cargo.lock, other crate or server edit,
mount behavior, foreign process termination, alternate target, clean,
workspace command, index operation, commit or push was performed by this task.
Root owns the subsequent coordinated index/commit window. The slice ends here;
there is no further feature scouting or GPU launch.


## Native tooltip columns, ratios and original ShowTooltip lifecycle (2026-10-25)

This slice starts after the accepted/pushed `46954e2c` checkpoint. Production is
confined to `src/lua/tooltip.rs`. The accepted Window `13d70922`, ListCtrl stable
keys/manual visibility/scriptSelect and caret `3487cae8` source contracts remain
unchanged. In particular, `src/render.rs`, `src/lua/edit.rs`, `src/lua/widget.rs`
and the ListCtrl sources are not edited. All new native research and validation
are in `data/extract/aa-ui/tooltip-columns-scout-20261025/`; earlier evidence is
not rewritten. The preceding caret documentation remains a historical record.

The original `scriptsbin/x2ui/baselib/tooltip/tooltip.alb` is freshly extracted
through `tools/aa-pak.exe`: 180035 bytes, SHA256
`58b4e057d2b35a360f5b5c5dee851f208b5dbe20ba24123971a6887922ede6bd`,
matching the retained original. Its unchanged main/67/0 skill-synergy section
uses right-column descriptions with ratios `.68` or `.88`; main/75 sell/repair
rows use `.35/.65` and `.4/.65`; main/50 reserves `.85` for a quest title and
adds a right-aligned status. These are actual display consumers, not new method
producers. `original-tooltip-alb-pins.json` retains their instruction hashes and
the original enclosing upvalue names. The new tests execute the original
instruction streams with controlled inputs; the full-window regression and
example additionally load the original base libraries, section constructors,
PushSection, SetInfo and ShowTooltip. They use the actual LARGE15/MIDDLE13
defaults and the loaded en_us `In-Progress` text. The isolated title test also
retains its explicitly controlled 18px style variant.

Native type-name lookup39276eb0 maps kind18 to `gametooltip`. Object factory
392a5f40 case18 allocates0x340 bytes and calls3929a480; Lua binding factory
39213840 case18 allocates0x50 bytes and calls3923eca0. The core constructor creates
two empty0x18-byte column deques at +0x2d0/+0x2e8, with vtable399c7cc8. Its +0x30 cast
is3929a520, and +0x154 draw is392987a0. Registration binds AddLine3923e110 through
3923e5a0/3923dde0 and AddAnotherSideLine3923e230 through3923e610/3923df20.
`tooltip-registration-chain.json` and `tooltip-type-registration-chain.txt`
retain this chain and exact native spans. The source DLL remains SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.

AddLine39299a80 selects the right column only for the exact C string `right`.
Its required text/font/column parameters are strings; font size, alignment and
indent are native f32-to-i32 CVTTSS2SI values. Alignment3/4/5 means left/center/
right; other values fall back to left, and center/right clear indentation.
The optional seventh argument starts at1 and changes only for the numeric
variant. Cry GetParam3550a460 and numeric conversion execute in the retained
oracle, with Lua stack/type/number primitives controlled. Numeric strings,
nil, booleans and tables do not become ratios. Raw f32 ratios are retained;
there is no invented ratio clamp.

For the used normal-width cases, native text layout receives physical inner
width times ratio, minus scaled indentation. Native39273b20 then subtracts ONE
physical pixel from its wrap limit. This supersedes the older capture-inferred
five-unit margin; it is not kept solely to preserve old assumptions. Native
draw392989d1..39298a5e aligns within the full ratio extent, separately from that
wrap limit: the right column begins at right edge minus inner width times ratio.
Right alignment ends at the right edge. Font request39299ac8/3929961b truncates
the scaled font size before layout. Measurement and actual text paint now use
the same physical request and wrap dimensions. Unwrapped fit uses the sum of
the widest left and right columns. The existing shared glyph painter is used
without editing it or adding a new Lua method/host hook.

Fresh current-source original-consumer REDs are `original-consumer-red.log`:
the original `.68` description paints at X110 instead of the native X184, and
the `.68/.88` branches both produce four rows. Those failures precede production
changes. Original quest-title, sell/repair money, emitted-glyph, physical-wrap,
width-refit, ClearLines and typed-argument checks accompany the correction.
Ratios0/negative/greater-than1 are checked as raw geometry/type inputs, not as
proof of all native typography at sub-glyph or nonfinite wrap widths.

Independent R1 was NOT CLEAR with two P2 findings. Its source/test/example v1
bytes are archived under `frozen-v1/`, and `production-test-freeze-v1.json` and
all v1 logs/captures remain intact. First, the example used GetSize for frame
extent, while original tooltip_view main/1/14 ShowTooltip computes an extent
from the actual first/last section offsets and includes the first inset twice.
For the reproduced first section, GetSize gives frame bottom244; the unchanged
original ShowTooltip recipe gives256. The example now calls
`ShowTooltip(true,nil)`, with no manual12px padding. The full constructor test
reproduced actual paint bottom258 versus frame244 before this correction.
It now checks the exact original extent recipe and `needShow`, separately from
the current-font ink bound. The remaining measured2px descent is explicitly
open native font/cap-height calibration; the test uses the current painter's
measured descent, and production adds no padding or baseline shift.

Second, native392997c1..39299856 chooses the opposite side from populated rich
row containers, not source-string nonemptiness. `r1-row-occupancy-red.log`
retains a current UiRuntime failure: right R, formatting-only left, then S
incorrectly displaces R. Admission now follows the parsed glyph/icon/image
layout, preserving transparent glyphs and explicit empty hard-break rows.
Native39273890 calls row creation39271570;392744bf consumes CRLF as one break.
The test checks emitted R/S, hard breaks, transparent glyphs, inline icons and
image fonts. The latter two are explicitly current-renderer-model admission
tests; they do not prove native empty-value markup or font-manager image-font
dispatch. The native293-case oracle supplies controlled row-container contents
and controls font lookup/layout/container endpoints: it proves the original
selection/arithmetic, not a complete original rich-text/FreeType implementation.

The separately authorized bounded getter correction follows39296740, executed
in27 additional pure native cases (`tooltip-height-native-cases.json`): empty
returns top inset in both outputs; nonempty include is current resolved height
minus bottom inset, and only additionally removes the last lower space. It reads
height instead of recomputing text. This constant top inset cancels in section
height differences and was not the example overflow cause. Only two absolute
expectations in the old `tests/game_tooltip.rs` migrate: empty0 becomes top10,
and only21 becomes10+21. Existing difference/spacing/fit assertions remain.
New checks cover empty/populated/spaced/Clear and explicit height changes.

The initial v1 full UI run is678 passing checks across96 targets, zero failures,
one pre-existing optional font-metrics ignore. It predates R1 corrections and
is not final-source validation. The focused corrected run passes12 new checks
plus3 existing game-tooltip checks. Final v2 full-suite/doctest/review/lint and
corrected-capture records are appended after they complete. No Clippy has run
or been queued before independent R2 settles.

The two original v1 diagnostic captures are
`shots/aa-ui/native-tooltip-columns-scale1-720-20261025.png` and
`shots/aa-ui/native-tooltip-columns-scale85-720-20261025.png`. Both selfclosed
with zero script errors and no actual renderer WARN/ERROR. FPS samples are
361.7/361.1/357.8/349.4 and353.1/310.0/358.5/331.8. `capture-proof-v1b.json`
correctly decodes the PowerShell UTF-16 logs; the earlier v1 metadata attempt
did not decode them and its empty FPS lists are not the measurement record.
The diagnostic images show the example sizing omission and scale.85 title/
status overlap. Neither is a completed image-parity claim. The separate
headless preflight has zero script errors but one existing Bevy no-RenderApp
ERROR and three WARNs; it is not substituted for a clean GPU capture.

Limits remain explicit: current FontLib shaping, native absolute font origin,
cap height/hinting, fallback/image-font manager semantics, exotic empty markup,
sub-glyph/nonfinite wrap kernels, inherited animation text re-rasterization and
geometry notifications outside the exercised content/width lifecycle are not
closed by this slice. The scale.85 title/status overlap is not attributed to
this diff or hidden by guessed ratios/fonts. General nonzero per-locale spacing
and rich-text corner cases retain their prior qualifications. Mounts, server
state, bag/cooldown authority, ModelView/FOV and held Grid+405 work are untouched.

The accepted caret blink/width/font-origin qualifications remain independent.
No dependency, manifest, Cargo.lock, other crate, server, accepted widget/render
source, foreign process, alternate target, cargo clean or workspace run is part
of this change. Root owns subsequent review/check release and the index window.


The frozen v2 example was rebuilt (exit0) and its copied executable ran exactly
two corrected, sequential selfclosing captures under root's new explicit grant:
`shots/aa-ui/native-tooltip-columns-scale1-720-v2-20261025.png` and
`shots/aa-ui/native-tooltip-columns-scale85-720-v2-20261025.png`. Both were
inspected and have zero script errors/WARN/ERROR. FPS samples are
301.8/299.1/291.5/269.7 and291.4/300.6/281.2/268.3; they are shared-machine
observations only. `capture-proof-v2.json` retains PNG/binary/source hashes and
correctly decoded raw-log results. Original ShowTooltip frame growth is visible,
with the current2px descender extent and scale.85 title/status overlap still
explicitly qualified. These are bounded diagnostics, not whole-font or original
client image-parity evidence. The GPU slot was released with no own process or
window left running; no further launch is planned.

Fresh original `tooltip_view.alb` is45038 bytes, SHA256
`dc70be3f07f6f67ae6bb3f47ab5c9135a8adfc752c86a151164e297125b2fabc`, matching
the retained asset. `original-tooltip-view-alb-pins.json` pins the original
constructor, PushSection, SetInfo, GetSize and ShowTooltip prototypes. The latter
is204 unchanged instructions with SHA256
`1b358cdbd88f3e89e1bb49acbdcb70c9fcb868a5483cb9bedf9bcd70c23d1978`.

`production-test-freeze-v2.json` pins four reviewed production/test/example
paths, including the two exact getter-fixture migrations. Final-source focused
validation passes15 checks (12 new plus3 existing tooltip tests), and doctests
pass2/2. All2331 previously accepted proof artifacts are verified unchanged in
`accepted-prior-proofs-verified.json`; `accepted-caret-evidence-pins.json` also
pins the retained caret slice read-only. The entire589738-byte historical
documentation prefix is preserved byte-for-byte. Fresh independent R2 is
pending; no Clippy has run or been queued before its result.

### Tooltip columns R2 scale refit and v3 freeze (2026-10-25)

Fresh independent R2 closed both R1 findings but found one P2: after a live UI
scale change, paint used the new physical font and wrapped rows while the
widget height and native-style getter retained their previous fitted height.
The v2 full suite (683 passed, zero failed, one preexisting ignore, 96 targets)
is interim validation, preceding this correction. All four exact v2 files were
archived under `data/extract/aa-ui/tooltip-columns-scout-20261025/frozen-v2/`;
the v2 pins, native evidence, logs and corrected screenshots remain unchanged.

`r2-live-scale-red-v3.log` demonstrates actual emitted paint in both directions:
the populated 15px `.68` right column changes from eight rows at `.85` to ten
at `1`, but its height stays `169.17647` until an unchanged `SetWidth` refits it
to `208`. The reverse paints eight rows with stale height `208`, corrected by
the same width call to `169.17647`. Getter outputs change with the stale height.

Native linkage is retained in the new `tooltip-scale-*-v3.txt` source spans and
pins. `3927b2f0` applies scale, calls vtable `+164` at `3927b3da..3927b3e2`, and
recurses through children without a visibility admission check. The game
tooltip vtable `399c7cc8` and unit-frame tooltip vtable `399ca0c8` both dispatch
`+164` to `3929a320 -> 39299ee0`. Unit-frame kind 19 resolves through factory
case `392a611d`, allocation `0x360`, constructor `392ce260`, base constructor
`3929a480`, then vtable `399ca0c8`. Reflow skips empty line deques at `39299f14`.
For populated lines it traverses both columns, requests truncated physical font
sizes at `3929a09b..3929a0a0`, rebuilds layout at `3929a182`, then calls
`39299130` at `3929a2c7`; height is written to `+78` at `39299585`.

`tooltip_scale_oracle.py` / `tooltip-scale-native-cases-v3.json` execute 24
original scale-dispatch cases across both real vtables, scales `.85/1/1.5`,
shown/hidden flags and empty/populated deques. Empty callbacks execute through
their original return and preserve logical manual height. Populated cases stop
at the first font refit call, proving dispatch and propagated scale; subsequent
font layout and height recomputation are source evidence, not a substituted
native font engine. Rust live-class traversal is tested separately rather than
inventing a native dead-object flag.

The bounded production addition is a private used tooltip `refit_scale` helper
and a `globals.rs` call only when `SetUIScale` changes the effective scale,
after releasing its toolkit borrow. It fits live, populated `gametooltip` and
`unitframetooltip` instances, including hidden ones. Empty, cleared and dead
tooltips retain manual sizing; same-scale calls retain populated manual height
and coherent getter outputs. Real scale changes refit populated manual height
as native reflow does. It does not run unit-value callbacks or resize unrelated
widgets. The native getter continues to read resolved height without refitting.

`r2-focused-final-v3.log` passes 19 tests: 16 column/runtime/original-consumer
tests and three existing game-tooltip tests. New regressions compare actual
equal-glyph paint rows and height/getter values before and after unchanged width
calls in both scale directions and both tooltip classes. The unchanged original
`tooltip.alb` main/50 quest consumer uses default LARGE 15 / MIDDLE 13 and
`In-Progress`, populates once, then remains coherent through `.85 -> 1 -> .85`.
Lifecycle controls cover hidden live widgets, dead objects, empty/cleared state,
same-scale manual sizing, unit-hook non-invocation and a synchronous Lua OnShow
handler entering the scale setter and re-entering its same-scale branch.

`production-test-freeze-v3.json` pins five production/test/example files.
`freeze-v3-preservation-verified.json` confirms all five pins, the archived v2,
all other starting owned sources, 2,331 older native proof files, 98 retained
caret proof files and the entire 589,738-byte historical documentation prefix.
The example and migrated getter fixture are byte-identical to v2. Accepted
render/edit/widget/ListCtrl source remains unchanged. Final full UI/doctests and
fresh independent R3 are pending; no Clippy has run or been queued.

The corrected two GPU captures remain v2 diagnostic evidence, not a capture of
the v3 library. No additional GPU process has been launched. Original frame
sizing `244 -> 256` is preserved, without a tuned 12px pad or baseline change.
The measured current-font 2px descender and `.85` quest title/status overlap
remain open and are not attributed to this diff. No whole-font or whole-tooltip
pixel parity claim is made; all earlier native-font, raw-ratio and rich-markup
endpoint qualifications continue to apply independently.

### Tooltip columns R3 retained-scale correction and v4 freeze (2026-10-25)

Fresh independent R3 closed the earlier findings but found one P2 in v3:
refitting every populated tooltip on a global UI scale change ignored an
`ApplyUIScale(false)` root or ancestor. Native scale remains retained in that
case. All five exact v3 files and the then-current documentation were archived
under `tooltip-columns-scout-20261025/frozen-v3/`. Its full suite of 687 passes,
zero failures and one ignore across 96 targets is **interim**, preceding this
correction; the v3 pins, raw logs and native proof files are unchanged.

`r3-retained-scale-red-v4b.log` contains two actual runtime failures. A frozen
15px tooltip retains effective scale `.85` but changes height `169.17647 -> 208`
after global scale becomes `1`. A complete original tooltip window's populated
native child changes height `124.7647 -> 129`. These tests were added before the
production correction; the earlier `r3-retained-scale-red-v4.log` is a retained
test-compilation diagnostic, not a runtime RED claim.

The original native chain is pinned in new `tooltip-apply-*-v4.txt` and
`tooltip-native-effective-scale-v4.txt` files. Registration at `3926a8df` binds
`ApplyUIScale` to `39265b90`; the typed boolean writes native `+2b6` at
`39265bb0`. Scale propagation `3927b2f0` writes `+68 = local +64 * parent +68`
in parent-first f32 order. Reflow's font update at `39299f1a`, explicit physical
font size at `3929a09b`, wrapping/layout at `3929a182` and height arithmetic all
use that effective scale. The getter divides resolved physical height by `+68`
at `39296774`, independently of global UI scale. Our tree stores local logical
height, so its existing resolved-height getter remains the corresponding local
result; it does not remeasure on query.

`tooltip-view-setinfo-v4.txt` records the unchanged original `tooltip_view.alb`
main/1/5 sequence: `ApplyUIScale(true)`, `SetScale(UIParent:GetUIScale())`, populate
sections, optionally choose a smaller fitting scale, then `ApplyUIScale(false)`.
This active full constructor path explains why a main/50 prototype alone was
insufficient to cover the retained-scale boundary. Original ALB hashes and
instruction pins remain unchanged.

`tooltip_retained_scale_oracle_v4.py` and
`tooltip-retained-scale-native-cases-v4.json` execute 40 native local/parent/global
scale cases through both actual tooltip vtables. Global `39ac67ec` varies
independently while the original `3927b2f0` / `39299ee0` path passes effective
local-times-parent scale to the font refit. As in the prior scale proof, populated
execution stops at that first font endpoint. It does not substitute native
font rasterization or text layout; subsequent native use is source evidence.

The v4 global hook snapshots live populated tooltip effective scales **before**
changing animation/global state, drops its toolkit borrow, then refits only IDs
whose effective scale changed. Hidden widgets remain included; frozen roots or
ancestors retain both fitted and manual height. `place` centrally obtains
`animations.widget_scale`, so fitting, side occupancy and row-count queries all
use the same native scale, including later AddLine/ClearLines/SetWidth calls.

Tooltip paint also lays out and rasterizes at effective scale. Its generated
geometry is converted back by `global/effective` to the existing painter's
pre-transform coordinates, allowing the existing widget/ancestor affine path
to apply once. Glyph atlas rasters therefore retain their physical dimensions
instead of being selected at the new global scale then stretched. This is
bounded to `tooltip.rs`; accepted `render.rs`, edit, widget and ListCtrl code
are byte-identical. No generic animation or widget behavior was changed.

`r3-focused-final-v4.log` passes **21 tests**: 18 column/runtime/original-consumer
tests and three existing tooltip tests. New controls cover frozen roots and
frozen ancestors for both game/unit-frame tooltip classes through `.85 -> 1 ->
.85`, same-width resizing and Clear/AddLine while global scale differs. They
compare actual emitted atlas UVs, physical glyph extents and relative column/row
positions, separately from qualified absolute font origins. All four complete
original synergy/sell/quest windows are populated **once** through original
constructors, SetInfo and ShowTooltip, then retain effective scale, height,
getters, row count and emitted paint through the global round trip. The earlier
non-frozen scale, hidden/dead/empty/manual-size, row occupancy and original
consumer controls continue to pass.

`production-test-freeze-v4.json` pins five source/test/example files. Preservation
verification additionally covers 203 older tooltip evidence files, 2,331 older
native proofs, 98 caret proofs, all other starting owned sources and the entire
589,738-byte historical documentation prefix. The example and getter fixture
remain byte-identical to v2/v3. Final v4 full UI/doctests and fresh independent R4
are pending; no Clippy has run or been queued, and no GPU/index action occurred.

Earlier font endpoint qualifications remain independent: this closes retained
effective-scale selection and current emitted geometry, not native absolute
font origin, cap-height/hinting/rasterization equivalence or general animation
timing/refit notifications. Nonpositive/nonfinite effective font kernels remain
outside the proved original positive-scale consumers; no new clamp is invented.
The measured 2px descender and `.85` quest title/status overlap remain open. No
padding, baseline, ratio or font-origin adjustment was made to hide them. The
two corrected v2 captures remain explicitly historical diagnostic evidence.

V4 final-source CPU validation completed: `runtime-suite-v4.log` records
**689 passed, zero failed, one preexisting ignore across 96 targets**, exit 0.
`doc-tests-v4.log` records **2/2 passed**, exit 0; focused
`r3-focused-final-v4.log` remains **21/21 passed**. Frozen five-file pins and all
older evidence preservation checks remain unchanged. Fresh independent R4 is
pending; the sole final Clippy has not run or been queued. Documentation may
receive validation-only additions after that review. No GPU or index action
was taken, and no commit/push authorization has been granted.

### Tooltip columns final v4 release validation (2026-10-25)

Root reported fresh independent **R4 QUALIFIED CLEAR**, with no actionable
P1/P2/P3 finding and all five reviewed source/test/example pins verified.
R1's original ShowTooltip sizing and rich-row occupancy findings, R2's live-scale
refit finding and R3's retained effective-scale finding are closed in this
bounded scope. Absolute font/rasterization, general animation notification and
the other explicitly retained endpoint limits remain qualified. This final
record supersedes earlier pending-review/check statements without rewriting
their historical evidence.

Final-source validation is **689 passed / zero failed / one preexisting ignore
across 96 UI targets**, plus **21/21 focused** and **2/2 doctests**. The sole
final lint invocation was:
`cargo clippy -p aa-ui --locked --lib --test tooltip_columns --test game_tooltip --example native_tooltip_columns`.
It exited 0 in 9.26s. There were 27 aa-ui warnings and one aa-dev warning; every
diagnostic source line maps to byte-identical HEAD source, including the two
globals warnings away from the new hook. There is **no changed-source
diagnostic**. The preexisting duplicate aa-client bin/example manifest warning
is separate. `clippy-diagnostics-final-v4.json` records the line mappings; no
second baseline lint invocation, autofix or cleanup was performed.

The final frozen example build exited 0 in 5.69s. The copied executable is
113,552,384 bytes, SHA256
`10e1f66eaa44378ac2bd3f46af4535786e5807a615e4d11da152865e80471aae`.
`example-binary-proof-final-v4.json` ties that exact copy to all five v4 pins.
Under root's serialized GPU grant, exactly two 1280x720 selfclosing runs used
that copy with `--shot-after 8`, sequentially:

| Final v4 capture | FPS samples | UI ready | Warnings / errors / script errors |
|---|---|---|---|
| `shots/aa-ui/native-tooltip-columns-scale1-720-final-v4-20261025.png` | 369.2 / 432.8 / 365.6 / 399.1 | 150ms | 0 / 0 / 0 |
| `shots/aa-ui/native-tooltip-columns-scale85-720-final-v4-20261025.png` | 348.6 / 360.7 / 366.6 / 370.3 | 109ms | 0 / 0 / 0 |

Both processes exited 0 and both images were inspected. The GPU slot was
explicitly released; no aa-ui QA process or window remains. The final PNGs are
byte-identical to the corrected v2 initial-scale images: scale 1 is 83,048 bytes,
SHA256 `86a0d40129c560092a7499e55700ef1154a931fbec4704cae120fda9d43ca914`;
scale `.85` is 69,571 bytes, SHA256
`1dae487a1cbc2e18e7382c6518d25d4b99df163a5810c0cf74a7ebd48be836a3`.
The final v4 live/global-retained-scale transitions are covered by the emitted
paint CPU regressions, rather than inferred from these initial-state shots.
`capture-proof-final-v4.json` records source/binary/image/log hashes and outcomes.

Original frame sizing `244 -> 256` remains unchanged. The historical measured
2px font-descender limit and visible `.85` quest title/status overlap remain
**open**, and no native absolute font or whole-tooltip pixel parity is claimed.
No padding, baseline, ratio or font-origin adjustment was made. The earlier v1,
v2 and v3 proof files, source snapshots and captures remain preserved.

The five reviewed production/test/example files remain frozen. Only this
validation documentation was appended after R4. Final six-path hashes,
preservation checks and index/lock readiness are recorded separately in
`ready-source-pins.json` and `validation-summary-final-v4.json`. No index,
commit or push action is authorized until root grants its exact-path window.


## Native ordinary TextBox physical rows and original speech bubbles (2026-10-25)

The unchanged `chat/chatbubble.alb` constructor and `SetChatMsg` /
`AdjustBubbleLayout` exposed a different visible sizing defect after the tooltip
font investigation. At UI scale0.85, `FONT_SIZE.LARGE=15` selects a physical13px
face. Two rows have physical height32, so `GetTextHeight()` must return
`32 / 0.85 + 3 = 40.647057` with the original chat locale's line spacing3.
The old getter used the logical15px face's18px height and returned39. The retained
pre-edit RED uses controlled spacing0 and records the same error as36 versus
37.647057. The corrected original consumer sizes its bubble from the physical
rows it paints.

This slice changes ordinary outline-font `textbox` widgets only. Textbox height,
line-count, longest-line-width and auto-height queries use the retained effective
scale, `FontStyle`'s physical font request (`integer size * scale + 0.3`, then
truncate), scaled insets, and the rich wrapper's one-physical-pixel breaking
allowance. Painting uses that same physical face and breaking mode, without a
second ancestor scale on the raster. The physical alignment extent still includes
the wrapping allowance. Public APIs and aa-client integration are unchanged.
Image-font TextBoxes keep their earlier path and qualification.

`TextBox392cddec` calls `FontStyle39275150` before parsing. SetText at
`392cdf79..392cdfb6` subtracts physical horizontal insets before
`39273c00 -> 39273b20` subtracts one pixel. Getter registration maps
`GetLineCount` to `3925c200 -> 392cd4f0`, `GetLongestLineWidth` to
`3925c2b0 -> 392cd510`, and `GetTextHeight` to `3925c360 -> 392cd650`.
The last two return physical measurements divided by native `+68`.
`392cd650` adds `(rowCount-1) * lineSpace * scale` to the physical font heights;
`392cd510` remeasures the stored row fragments through FontStyle/FontCore.

The TextBox-only layout mode also follows the native pending-word/separator
state in `392716c0`. Admission checks a word with its following separator,
retains native empty rows when narrow separators exhaust a line, and handles
leading separators, CR/LF/CRLF and tabs in that mode. Kerning used for admission
belongs to the incoming character; a painted/re-measured row starts its own
kerning sequence. Plain text, edits and tooltip layouts retain their previous
breaking modes and have separate cache identities.

TextBox `SetSnap` registration `3925b96c` binds `3925b090`, whose valid receiver
calls `392748c0` (style bit2). `DrawFont39221335/39` applies f32 `+0.5` and signed
truncation to each physical glyph origin, including fractional inter-row spacing.
This is now observed by TextBox painting. Snap-disabled origins retain fractions.
No other class's snapping, edit/caret behavior, Window, ListCtrl or tooltip
contract is changed. Existing broader font, rasterization and absolute-origin
qualifications are retained.

### Evidence and disconfirming tooltip controls

All new research is in ignored `data/extract/aa-ui/font-layout-20261025/`.
The read-only native source is `<client12-unpacked>/x2game_unpacked.dll`, SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
Targeted pak extracts confirm that en_us's default logical font selects
`ui/font/flareserif_821_roman.ttf`, SHA256
`3a864bfc346533f81863fe8663769805e198e6ab02f8e837c4099bebb735f75d`.
The CPU harness executes the retained client's actual FreeType2.1.10 library,
real memory face, size setup and `0x8004` glyph loads. Allocation, byte-memory /
string conversion, successful setjmp and STL bookkeeping are explicit endpoints;
font metrics, glyph loading/hinting, kerning and rich word wrapping are not metric
stubs. The alias manager and GPU texture upload are outside this harness.

`native-font-380-real-glyphs.json` records380 ASCII glyphs at11/12/13/15px.
All advances match the font's authored `hdmx`; only six15px symbol/`J` advances
differ from rounded unhinted metrics, none in the tooltip sample. Font ascender /
height controls match current FontLib at those sizes. No font substitution,
FreeType dependency, padding or guessed baseline was introduced.

`native-textbox-matrix-v2.json` retains132 real-font/native-wrapper cases,
including native row texts and their native FontCore remeasurement.
The corresponding checked-in `tests/fixtures/textbox-native-rows.tsv` drives the
actual runtime getters. `native-bubble-controls.json` retains the four scale
controls for the unchanged speech-bubble text. `native-textbox-snap-24-cases.json`
executes24 unchanged native glyph-position cases with explicitly controlled
bitmap metrics; it does not claim native coverage parity.

The original tooltip investigation was disconfirming. For the exact artificial
quest title used by `native_tooltip_columns`, native wrapping at scale0.85 puts
`Old forests conceal distant` on the first row (140px). The actual en_us
`In-Progress` string is54px at11px. The178.5px inner extent places the status at
124.5px, permitting15.5px of horizontal overlap with the title. At scale1,
the native first title row is129px and the status starts at145px (16px gap).
`tests/font_layout.rs` keeps passing controls for these measured advances.
The authored ratio is not changed to hide the overlap. The previously measured
2px descender overdraw remains qualified: native/Swash bitmap coverage and full
absolute tooltip origins are not established as identical, so no frame padding
or baseline shift is claimed to fix it. All accepted tooltip and caret proofs
remain immutable.

### Validation

Final own-suite, capture and independent-review results are recorded below after
source freeze. The visual example `native_textbox_layout` calls unchanged full
original constructors and setters, loads the original ko/en_us chat locales
(including spacing3), and controls only game speaker/projection data.


### R1 response: populated TextBox auto-height on effective scale changes (v6)

The v5 source freeze and its single `.85` original three-bubble capture are
**interim**. Independent R1 review found one P2: a populated auto-sized TextBox
remeasured/painted its current physical rows after a scale change, but retained
its old stored height. The actual UiRuntime RED used the original bubble's
225-wide, LARGE15, line-space3 sentence. Both global and ancestor scale changes
left height39 at `.85`, where the native row/font contract requires40.647057.
The retained RED log is `r1-auto-scale-red-v5.log` (two failures, exit101), and
`textbox-scale-red-source-v5.rs` preserves the exact pre-fix test source.
The earlier actual original-chat-bubble RED and native separator edge remain
retained; v6 does not broaden the accepted wrapping or font scope.

Native `392cddd0` updates the physical font/rows, reads the auto-size flag, and
calls `392cdca0`. The latter retains the flag, skips a false flag, and computes
height from native row count, actual FreeType font height, scaled line spacing
and physical top/bottom insets. `native-autoheight-10-cases-v6.json` executes ten
native auto-enabled/disabled controls at scales1, .85, 1, .7 and1.25, using the
same real-font harness. Only the final Window height-storage/notification call
is captured as an endpoint; this does not establish broader Window callbacks.

The correction indexes populated, auto-sized ordinary TextBoxes at existing
content/auto-size paths and remembers their applied effective scale. Native
completion and callback entry compare that scale with the current inherited
scale before returning to script. A changed scale refits the stored height
using the existing physical TextBox layout. This pass calls no Lua; unchanged
scales do not clone/layout text or overwrite a manually assigned height.
GetHeight and measurement getters do not own invalidation. Opted-out roots and
frozen parents therefore retain their heights across global changes when their
effective scale is unchanged. Disabling auto-size, clearing text, changing away
from ordinary font layout or retiring a widget removes its indexed entry.
The new empty/dead guard preserves the requested bounded behavior; it does not
claim the native empty-row auto-size contract is fully implemented.

`tests/textbox_scale.rs` reads stored tree heights before any size query and
exercises populated1 -> .85 ->1 through global and ancestor setters, callback
observation, opted-out/frozen controls, hidden widgets, manual/disabled heights,
empty/dead state and callback reentry. The accepted five tooltip sources remain
byte-identical. Existing font/baseline qualifications,132 native wrapping cases,
physical snapping controls, caret, image-font TextBox, other classes, Window and
ListCtrl behavior remain outside this correction. Final v6 pins, validation
results and the fresh independent-review brief live in the ignored research
folder. No new GPU launch is authorized by this correction.


### R2 response: refit before OnChangedAnchor geometry snapshots (v7)

The v6 freeze is also **interim**. Independent R2 review found one P2: callback
entry refit ran after `begin_anchor_update` had retained the receiver's old
rectangle. Thus OnScale saw the updated height, but the earlier OnChangedAnchor
could copy stale GetHeight into another widget. R2 reported no other P1/P3 and
verified the12 v6 pins and immutable v5 artifacts.

The actual runtime RED is `r2-anchor-scale-red-v6.log` (one failure, exit101),
with exact source `textbox-scale-r2-red-source-v6.rs`. For the same populated
225-wide15px/spacing3 TextBox and parent:SetScale(.85), OnChangedAnchor read39
and copied39 into a partner while the raw TextBox height was40.647057. This was
a runtime consumer failure, beyond checking only a producer or OnScale.

The correction invokes the existing bounded TextBox refit at anchor propagation
entry, before `begin_anchor_update` takes the affected geometry snapshot. The
existing synchronous propagation, callback order and notification rules remain
in use; no events or query-owned resizing are introduced. Completion/callback
checks remain for other scale completion paths. The regression copies the
observed height into a real partner and checks both through .85 ->1. The prior
six R1 controls stay in the same suite. All font/raster/wrapping qualifications,
protected tooltip sources and earlier source/proof/documentation bytes remain
preserved. `interim-v6-status.json` records preservation before unfreezing;
the superseded v6 full-test compile was canceled before test execution, so it
has no full-suite pass claim. Final v7 pins and the R3 brief include both review
findings and the original native TextBox scope.


### R3 response: enabled-auto nested scale inside an active anchor callback (v8)

Fresh R3 found the remaining nested form of R2: the initial snapshot could be
correct, but a nested UIParent:SetUIScale inside OnChangedAnchor changed the
physical font/auto-height while the active retained rectangle stayed stale.
The earlier reentry control disabled auto-height before nested scaling and did
not cover this enabled-auto consumer. R3 reported no other P1/P3 and verified
all13 v7 pins, protected sources, documentation prefixes and old artifacts.

`r3-nested-anchor-red-v7.log` executes this actual runtime consumer (one failure,
exit101); `textbox-scale-r3-red-source-v7.rs` retains its source. With auto-height
enabled, parent:SetScale(1) enters OnChangedAnchor, which sets the global scale
to.85 and copies box:GetHeight into a real partner. Both observed/copied values
were39 although raw height was40.647057, matching R3's source-derived finding.
The superseded v7 full-test compile was canceled before runtime tests, without
a full-pass claim. `interim-v7-status.json` records its preserved artifacts and
all source pins were verified before unfreezing.

When the existing ordinary TextBox auto-size helper changes stored height, it
now refreshes geometry through the existing Tree::begin_anchor_update and
end_anchor_update path. This preserves its readiness guard and retained layout
resolution, adds no Lua calls or anchor event dispatch, and lets an already
active callback observe the changed height before copying it. There is no
getter-owned resize or new Tree API. The prior pre-snapshot refit stays in place.
The new enabled-auto nested-global regression checks both.85 and1, copied
partner height and exactly two callback entries, alongside all seven earlier
R1/R2 controls. No other class/font/wrap/baseline behavior is broadened.
The v8 freeze and complete R4 brief retain the original native TextBox contract
and all three review findings, responses, runtime REDs and proof limits.


### R4 response: retained anchor dependents of a nested auto-height change (v9)

Fresh R4 found one P2 remaining beyond the direct receiver: depth-first anchor
propagation had already retained a stretched dependent's old rectangle when the
source TextBox callback performed a nested global scale change. Refreshing only
the TextBox left dependent:GetHeight stale until the outer dispatch ended.
R4 closed the prior direct-receiver cases and reported no other P1/P3; all13v8
pins, protected sources, old5520 proofs and documentation prefixes matched.

`r4-dependent-anchor-red-v8.log` executes the actual failure (one failure,
exit101); `textbox-scale-r4-red-source-v8.rs` preserves the source. The existing
225-wide15px/spacing3 fixture has a TOPLEFT/BOTTOMRIGHT dependent and a second
stretched transitive leaf. Inside box.OnChangedAnchor, nested global.85 made
rawbox40.647057, while dependent, leaf and both actual partner copies were39.
`interim-v8-status.json` preserves exact source/proof/copy artifacts before
unfreezing. No superseded full-suite compile is counted as a full pass.

The existing changed-height ordinary TextBox helper now refreshes its source
and the affected anchor-dependent vector recursively through Tree's existing
begin/end_anchor_update geometry path. Each node checks current liveness and
the existing update/readiness guard; pending targets are not promoted by this
refresh. It follows dependency-vector order without Lua, notifications or a
synthetic callback. It does not clear retained geometry globally or resize in
queries. All other class layouts and event propagation remain in their prior
paths. The regression verifies both direct/transitive observed and copied
heights through.85 ->1, plus the exact existing depth-first callback trace
LDBLDB. All eight prior R1/R2/R3 controls remain passing in the nine-test suite.
The v9 freeze and complete fresh R5 brief retain the original native TextBox
contract, every prior finding/response/runtime RED and all proof limits.


### Final v9 validation and fresh R5 clearance (2026-10-25)

Fresh independent R5 review is CLEAR (no P1/P2/P3). It verified all13 frozen
source pins, protected5 tooltip sources, historical documentation prefixes and
all5520 prior research proofs; R1-R4 are closed, including dependency order,
retained offsets, liveness and absence of added events. All nine new runtime
regression/control cases pass, including the exact LDBLDB callback trace.

The final `cargo test -p aa-ui --tests` exited0:706 passed,0 failed,1 ignored
across99 targets. The existing ignored `text::tests::font_metrics` is a print-only
pak font diagnostic, not a newly expected RED. The frozen-source focused suite
had77 passes across9 targets, doc tests had2 passes, and the original speech
bubble example build exited0. No source/test edits followed R5 clearance.

Exactly one final Clippy command ran after review settlement:
`cargo clippy -p aa-ui --lib --tests --example native_textbox_layout --message-format=json`.
It exited0 with142 emitted compiler warning diagnostics,115 unique warnings
and0 errors. This is not a warning-free or baseline-clean claim. The actual
warnings include argument counts in TextBox layout methods, duplicated fake
modules in textbox_layout, float precision and other crate/test/dependency
lints. Full JSON and rendered diagnostics are retained as
`clippy-final-v9-diagnostics.json` and `.txt`; the aa-client duplicate-target
manifest warning is recorded separately. No warning cleanup or second Clippy
run changed the reviewed bytes.

The sole authorized final visual launch used copied frozenv9 executable SHA256
`839250df61ab018e439e75e0a37c4d44d74fd9d001dc4ec003f0ee5f876b1331`,
scale.85,1280x720, screenshot-and-self-close after6s. The inspected final capture
is `shots/aa-ui/native-textbox-layout-scale85-720-final-v9-20261025.png`, SHA256
`b102aed650f30b86fd6e337d1340a4b3d07a82679bfbc5ce6d347fac6ff8996a`.
All three original bubbles contain readable text; this newly rendered static
fixture is byte-identical to the interimv5 image. Actual FPS450.8/359.2/375.3,
UI ready101ms, engine WARN0/ERROR0 and both script reports0. The process ended
and the GPU slot was released promptly. PowerShell did not retain its OS exit
code after WaitForExit (the exit file is empty); successful screenshot/self-close
and process absence are verified, but an exit0 is not claimed. Targeted read-only
exit-audit recovery was unauthorized; no escalation or extra launch occurred.
The raw logs, diagnostic and image/executable hashes are retained in
`gpu-scale85-final-v9-summary.json`.

Only this validation section was appended after the reviewed freeze. All12
non-documentation owned files remain byte-identical; the complete pre-results
v9 documentation and all older prefixes/proofs are preserved. Final13-file
hashes are in `ready-source-pins-v9.json`, with full checks/diagnostics/boundaries
in `validation-summary-final-v9.json`. The original native tooltip overlap and
font/raster/absolute-origin qualifications above remain unchanged. This agent
performed no index, commit or push action; Root owns final integration/commit.


## Positive-page Slider track presses from original Sound Options (2026-10-25)

This slice follows the unchanged original CreateOptionFrame -> Sound Options
CreateSoundQualitySlider -> CreateOptionSliderBar -> CreateSlider path. The
original quality control explicitly uses range1..4, step1, page1 and its17x26
thumb skin. Its original Init/Save continue through existing X2Option authority;
no settings API or chat-window persistence code is added. Eight selected ALBs
were freshly matched to the pak. Grid+405 remains unproved and is not involved.

The actual pre-edit Runtime RED loads the full unchanged original constructor,
seeds quality2 through the existing option provider, runs original Init and uses
real hit-tested physical pointer input. It recorded press4/originalSave4/held1/
release1 with2 callbacks, versus native3/3/3/3 with1. The initial failed test and
old production hashes are retained in active-next-20261025-r1. Production edits
began only after this meaningful RED. The original consumer now passes.

Native Slider admission392c20a0 checks BOTH Slider and thumb IsEnabled through
392778f0 before press392c2a80. This gate is separate from the raw press body.
Press392c2a80 uses inclusive thumb rectangle classification392757c0 over actual
physical origin+6c/extent+74. For positive PageStep, track392c2830 applies one
clamped value +/-page by axis and leaves slider capture inactive. Native core
392c24b0 emits SliderChanged23 even when clamping leaves the value unchanged.
Typed PageStep adapter39255a90/39255920 ->39254e80 ->392c1f80 requires one float;
class init392c1e40 gives page0. The actual consumer explicitly sets page1.

A bounded CPU oracle executes the retained native cast/typed reader/initializer,
scalar setter, inclusive classifier, page/axis arithmetic and clamp. Its56
controls pass. Supplied physical rectangles and function-handler interface,
hit-status/art virtual, Lua/internal32 delivery, capture manager, final thumb
update and base-move calls are explicit controlled endpoints. It does not claim
native transform-chain execution, complete dispatch or callback geometry parity.

The implementation is a private used positive-page Slider-receiver press path.
It rechecks current liveness/visibility and Slider/thumb enable state, compares
effective geometry and the pointer in one physical space with inclusive edges,
commits a page without value-step snapping and queues exactly one change notice,
including unchanged clamps. Track presses do not retain a slider drag. A direct
Slider receiver hit on an inclusive thumb edge skips the initial centred jump
and uses the existing capture path; physical ThumbButton routing is unchanged.
The existing release, clipping, fade handling and scroll bridge remain in use.

Generic set_slider notification rules and public SetValue ABI, zero-page input,
physical ThumbButton routing, broader native callback-before-thumb-refresh
geometry and generic dispatch remain qualified. No generic setter, timer,
capture system, geometry event flow or other widget-class implementation is
rewritten. TextBox/tooltip/edit/Window/ListCtrl source/proof history and the
native raster/absolute-font-origin qualifications remain preserved.

The first auxiliary control run had three fixture failures: two deliberately
constructed top-level test hosts were not shown, and a frozen-scale assertion
incorrectly required absolute origins to remain fixed. The fixture response
shows those hosts and checks preserved extents plus actual effective pointer
geometry, without a production geometry change. The final focused suite has
10 passes: full original Sound/Init/Save, original ScrollWindow viewport/anchor
bridge, global/ancestor/frozen axes and inclusive edges, fraction/unchanged clamp,
off-axis no-page behavior, clip/disable admission, callback hide/retire/reentry,
release completion and the Sound-only visual fixture.

The final visual example source uses the same complete original Sound subframe
inside a controlled original window, avoiding construction of unrelated option
pages during a short capture. It is selfchecking through real input and original
Save and uses aa_dev default plugins/DevPlugin. No GPU launch is authorized yet.
Exact R1 source pins and the complete fresh review brief live in ignored
active-next-20261025-r1. Protected checks run against the frozen code; final
Clippy and visual launch remain gated on independent review/Root coordination.


### R1 visual startup failure and R2 example readiness correction

The fresh independent R1 review returned QUALIFIED CLEAR for the bounded positive-page
path, with no P1/P2/P3 finding and all six source/protected pins verified. The first
actual GPU launch of the frozen R1 copied example then exited with OS code 101 in
2.19 seconds, before producing a PNG or FPS samples. The retained Python process
handle and wait result record that exit. Bevy reported a system parameter validation
panic, `Non-send data not found`: the example required `NonSendMut<UiRuntime>` before
`UiPlugin`'s Update initialization had installed it. The log separately reported UI
ready with 998 objects, zero script errors and scale .85. Zero WARN/ERROR log records
do not make that panic a successful run; no visual pass or screenshot is claimed.
The R1 six-file archive, executable and failed-launch provenance remain immutable in
`active-next-20261025-r1/`.

Root authorized only an example readiness correction: `page_once` accepts
`Option<NonSendMut<UiRuntime>>` and returns while absent, before consuming its local
`done` state. This follows the existing toolkit's asynchronous runtime initialization
contract. Production input, private page handling, Runtime tests and the original
fixture are byte-identical to R1. R2 refreezes those four unchanged paths plus the
corrected example and this append-only documentation. The active full UI test run
may validate the unchanged production/test targets; it does not prove this example
launch. A fresh R2 review and corrected example build are pending at this source
freeze. No corrected GPU launch or Clippy has occurred yet.


### Final R2 validation and delivery evidence

Fresh independent R2 review returned QUALIFIED CLEAR, no P1/P2/P3, with all six
current source/archive pins verified. The only R2 code difference from R1 is the
example's optional-runtime readiness guard. The final `cargo test -p aa-ui --jobs 1
--tests` completed with Cargo 0: 716 passed, zero failed, one ignored, across 100
targets. Production input, private page handling, tests and the original fixture
were unchanged throughout that run; it does not itself validate the corrected
example launch. `cargo test -p aa-ui --jobs 1 --doc` completed with Cargo 0 and two
passed doctests. The focused original-consumer matrix remains 10/10 passed and the
bounded native CPU oracle 56/56 passed with its stated controlled endpoints.

The corrected frozen example built with Cargo 0 and was copied to
`qa-native-slider-page-r2.exe`, 113705984 bytes, SHA256
`12ffa7d512881b88a10877114b1c9eb7d5e05e3b5dda2e869308b7216564a404`.
Its single authorized corrected capture used .85 UI scale, 1280x720, shot-after 6,
and Python Popen/wait retaining the actual process handle. PID 17604 self-closed
with actual OS exit 0 in 9.33 seconds. The inspected PNG shows the original Sound
Quality thumb at High, the third label. The actual runtime assertion reports
original Init 2 -> real pointer page 3, original Save 3, one callback, and held
movement without capture. FPS samples were 396.2, 448.8 and 387.2; actual engine
WARN/ERROR records and reported script errors were zero, with no panic. The PNG is
`native-slider-page-scale85-720-final-r2.png`, 97492 bytes, SHA256
`f279c7171964d250aa1aae8e325f647ac54729d500558194fb48d955f7213104`.
This is a controlled original Sound host capture, not full native raster/origin or
whole-options layout equivalence. The failed historical R1 launch remains retained
as actual OS 101, no PNG/FPS, startup parameter panic; its zero WARN/ERROR records
were not a successful run. Both old and corrected executable/provenance remain.

Exactly one final scoped Clippy was run after R2 settled:
`cargo clippy -p aa-ui --jobs 1 --lib --tests --example native_slider_page
--message-format=json`. It completed with actual OS exit 0 in 56.91 seconds,
zero error diagnostics, 143 emitted warning diagnostics and 116 unique warnings by
code/message/primary location. The owned-path diagnostics are a collapsible `if`
in input.rs and the unused fixture `install` helper. Other crate diagnostics remain
in the retained JSON, and the Cargo duplicate bin/example manifest warning remains
in stderr separately. This is an observed warning report, not a baseline-clean
claim. No annotation cleanup, code/test edits, second Clippy or redundant test run
followed. The UI compiler batch and GPU slot were explicitly released; the AAEmu
peer RAM window is active and no further compiler/GPU launch is queued.

All five code/test/fixture/example files remain byte-identical to reviewed R2.
Only this factual validation append changes the docs pin. The full R1 and R2 source
archives, prior TextBox twelve non-doc pins, complete pre-task docs prefix and old
failed-launch evidence were verified without rewriting old proofs. Generic
SetValue ABI/default/changed-only behavior, zero-page and actual ThumbButton routing,
native callback-before-thumb-refresh geometry and broader dispatch remain held and
qualified. No index, commit, push, foreign source or new feature was authorized.
Final six-path raw pins and log/copy/capture/preservation proofs are retained in
`ready-source-pins-r2.json`, `validation-summary-final-r2.json` and
`ready-evidence-pins-r2.json` under `active-next-20261025-r1/`.


## Slider range-clamp notifications in the original upgrade notifier (2026-10-26)

The used original `SKILL_UPGRADED(id)` notifier exposed a range-reset gap. Current
Root source decodes its skill-upgrade event and emits one numeric skill ID after
HUD synchronization. The original skill toc loads `skill_info_notify.alb`; its
handler calls ShowNotifyer/AddSkill/AddChild/RelocateChildren/UpdateScroll. The
original UpdateScroll first resets the range to zero, then measures actual child
offsets and extents, then rebuilds the range. Native Slider SetMinMaxValues commits
the clamp and delivers its changed-value OnSliderChanged before that Lua measurement.
The old binding clamped silently, leaving the content displaced during measurement.
This source-to-consumer join is qualified current-host evidence, not a new native
packet/live-session parity claim. NEW_SKILL_POINT has no newly proved Root producer.
LEVEL_CHANGED remains a separately guarded original consumer, adding at levels5/10
with a matching script unit ID and newly pushed level state.

The actual pre-edit Runtime test loaded the complete unchanged original constructor,
locale/helper/action slot and existing16064 supplier, then drove real wheel input.
All admissions and zero-script-error checks passed. Eight to nine distinct original
children with old displacement40 produced stored maximum351 versus actual zero-origin
expected391. At that stored maximum the final entry bottom205.66666 exceeded the
viewport175.00003. That was one actual behavior failure/Cargo OS101, not a setup or
compile failure. Original offsets/heights were queried; no50px TextBox height or
font padding was assumed. The failed sources, native bytes, logs and process proofs
remain in the ignored `active-next-20261026-r1/` evidence directory.

One bounded actual-native batch passed20 controls under declared endpoints: real
range adapter/wrapper/class cast/typed readers/clamp/event ordering, with function-
handler interface endpoints and RECORD-ONLY Lua callback/geometry endpoints. Native
mapped memory was31207424 bytes under a96MiB emulated-map cap. It proved changed/no-
change, defaulttrue/false, hidden/disabled programmatic calls and finite reversed
bounds. The latter was initially informational; Root explicitly added same-setter
finite normalization and required two separate actual Runtime goldens before any
production change. Old40 returned[10,10,10,0] instead of[0,10,10,1], and old5 returned
[10,10,10,0] instead of[0,10,5,0]; both failed on unchanged production. The finite
normalization now uses the exact native comparisons, selecting argument two for
both bounds on finite ties. This does not broaden nonfinite, invalid-FH, signed-zero
clamp, native ownership or callback-visible thumb-geometry claims.

Only the existing Slider SetMinMaxValues block changed in production. The used
finite numeric path commits bounds/value and queues one changed-clamp callback
unless an explicit Booleanfalse suppresses it. The existing method wrapper flushes
once the mutation borrow ends, before Lua resumes. The legacy compatibility path
outside those used numeric inputs remains held; generic SetValue, zero-page and
ThumbButton input, Tree/anchors/render/fonts/listctrl and Root APIs were not changed.
The widget file's complete raw prefix and suffix outside this range block match the
pre-edit archive. No synthetic internal32 or geometry event was added.

Focused R5 was9 passed/3 failed because new controls incorrectly assumed immediate
retirement after hide. Existing frame cleanup performs removal after callbacks.
Only new test timing/observations were corrected, preserving valid hidden-live calls,
then checking dead guards after the existing cleanup. Focused R6 passed12 tests,
including notification policy, finite goldens, liveness/reentry, LEVEL_CHANGED,
singleton recreation, exact page-to-range callback records and the .85 visual
fixture. The original upgrade now stores391=expected391; the last entry bottom is
165.66669 within viewport175.00003, reachable, with zero script errors.

The protected thirteen-target run completed173 passed/1 failed/1 ignored. Its sole
failure was the old page test observer being overwritten by the new legitimate
range callback: final seenValue1 versus the old3 assertion, before its count assertion.
Root explicitly authorized only that old test function to retain strict first-page
observations3, record every callback with unchanged action execution, and require
range-only second record[1,0,1,1]/count2; other actions remain exact1 and all existing
release/dead/held/final-value checks remain. The complete old accepted test and failure
are archived; all bytes outside the target function are unchanged. A preparation
selector failure mistakenly followed by an old-source Cargo launch was reported and
retained as repeated old-source failure, not fix validation. The corrected file was
independently verified by Root, and the replacement runner asserted its approved raw
SHA and outside-function equality BEFORE Popen. That actual replacement run passed
all10 page tests and all12 range tests, Cargo OS0. Other protected checks were retained
on unchanged code; no blanket protected rerun or full-suite pass is claimed.

The original-notifier example uses default_plugins/DevPlugin, real wheel/new-upgrade/
wheel-to-bottom, and an optional-runtime guard before consuming its done state. Its
shared .85 scenario passed CPU checks. The example-only build completed OS0 and was
copied as `qa-native-slider-range-r1.exe`,113730560 bytes, SHA256
`5f765cdb993707fb75a1b469e1e1bdfbe6db048a9acaed5315dac50f54512be5`.
It has NOT been launched; no GPU image/FPS/runtime-log success is claimed. The build
reported the workspace duplicate bin/example manifest warning. No Clippy has run for
this range task; full tests and the single final Clippy await fresh independent
review settlement, and GPU use needs a separate Root grant. Exact six source paths
are frozen for Root's fresh R1 review; no staging/commit/push or child delegation.


### 2026-10-26: final bounded Slider range validation after R1 qualified clearance

Root reported fresh independent `next-20261026-ui-slider-range-review-r1` QUALIFIED
CLEAR with no P1/P2/P3 and exact six start/end pins. The five reviewed code files
remain byte-identical. This section is only a factual validation append; the full
649068-byte reviewed document prefix is retained unchanged.

The serialized final CPU batch completed on those frozen bytes:

- `cargo test -p aa-ui --jobs 1 --tests`: actual OS0, PID30732 dead, 517.96s;
  728 passed, 0 failed, 1 ignored across 101 result targets. The retained ignored
  test is `text::tests::font_metrics`; no new ignore was introduced.
- `cargo test -p aa-ui --jobs 1 --doc`: actual OS0, PID32176 dead, 9.55s;
  2 passed, 0 failed, 0 ignored.
- Exactly one `cargo clippy -p aa-ui --jobs 1 --lib --tests --example
  native_slider_range --message-format=json`: actual OS0, PID18964 dead, 65.23s.
  Raw JSON/stderr retain 142 warning emissions / 115 unique diagnostics, deduplicated
  by package, code, message and primary spans across target repeats. None has a
  primary span on the six owned paths. The aa-client duplicate bin/example manifest
  warning also occurred. This is not a baseline-clean or new-versus-existing claim;
  no warning cleanup, annotations or second Clippy run followed. The compiler slot
  was released immediately at actual terminal.

Root separately granted exactly one copied-executable capture, with every source,
archive, CPU-result and executable hash checked before Python Popen. The frozen
113730560-byte `qa-native-slider-range-r1.exe` has SHA256
`5f765cdb993707fb75a1b469e1e1bdfbe6db048a9acaed5315dac50f54512be5`.
It ran the unchanged original notifier with the real wheel/new SKILL_UPGRADED16064
scenario at .85, 1280x720, `--shot-after 6`. Python wait returned actual OS0,
PID19896, 9.91s; independent process lookup confirmed absence. GPU ownership was
released before image/log inspection, with no retry or second launch.

Actual original range/bottom assertions logged old displacement40, stored max391,
expected max391, last bottom165.66669 <= clip height175.00003, reachable=true.
The inspected PNG shows the final entry's icon and both text lines above Confirm;
the scrollbar/thumb stays inside the notifier and its frame stays inside the image.
This proves the bounded current containment/reachability scenario, not native font,
raster, absolute-origin or pixel-identical reference parity. FPS was456.3,425.1,397.9.
Full stdout has0 WARN/0 ERROR/0 panic/0 script-failure lines; stderr is empty. Startup
reported0 script errors atscale.85. No separately unlogged final error-counter claim
is made. PNG `shots/aa-ui/native-slider-range-scale85-720-final-r1-20261026.png`
is63330bytes, SHA256
`f934d296a7fef67ced87a862de8906f2b5f6761196e5a13b533639f4803c3e86`.

Final records are under `data/extract/aa-ui/active-next-20261026-r1/`:
`cpu-validation-final-r1.json`, `visual-final-r1-inspection.json`,
`example-execution-final-r1.json`, `ready-source-pins-final-r1.json`, and
`ready-evidence-pins-final-r1.json`. Immutable `example-copy-r1.json` describes
status_at_copy_time; its unlaunched GPU field is a historical copy-time snapshot,
not the current execution status. The final execution record explicitly labels both.

All earlier proof remains: original40-deficit Runtime RED, two Root-authorized
finite-reversed REDs, native20 endpoint-qualified controls, R5 delayed-removal
fixture failures and fixture-only correction, R7's first seenValue1vs3 observer
failure, R8's failed edit/old-source mislaunch and superseded announcement, actual
R9 correction/Root approval, and R10 strict22PASS replacement. The prior93 evidence
pins and277 unaffected tracked files were verified unchanged; widget/page bytes
outside their exact approved blocks remain exact. Original host delivery is still
source plus Runtime sink evidence, not live/native packet parity. NEW_SKILL_POINT
bridge, generic SetValue, zero-page/ThumbButton/general dispatch, nonfinite/invalidFH,
signedzero-clamp/native ownership and callback-visible thumb geometry remain held.
No new source feature, build, native oracle, child, index mutation, commit or push
was performed for this final validation.


### 2026-10-26: ordinary Slider.SetValue explicit-Boolean notification (authoring freeze)

This is the separate ordinary public SetValue contract used by unchanged original
Sound volume Init. It is not the native changed-only range-clamp contract committed
in6661. Only a finite float32 numeric value plus an EXPLICIT Boolean enters the new
private used path, with finite ordered bounds and a finite stored value. It commits
the clamped value and queues one OnSliderChanged whenever the flag is true, even
unchanged or unchanged after clamp. False commits without an event. The existing
method wrapper flushes after the UI borrow is released and before Lua resumes.
There is no visibility/enabled admission gate or new public API/state/event.

The existing input/shared set_slider helper remains byte-identical and changed-only.
The entire6661 SetMinMaxValues block is byte-identical. Outside the declared finite
explicit-Boolean domain, the public binding calls the old helper with its existing
number/default/Boolean conversions. The private path validates through immutable
widget access before any mutation, so invalid-state fallback adds no extra Tree
snapshot/mutation. Missing flag, numeric strings, nonnumeric/nonfinite inputs, invalid
bounds, signed-zero clamp/native coercion semantics remain legacy and qualified;
this is not the known missing-required-flag ABI cleanup. Input, positive PageStep,
zero-page/ThumbButton routing, internal32 bridge and callback-visible thumb geometry
are not changed or claimed.

The native evidence is deliberately split. Historical native-range-oracle-r3.json
SHA0127a7fc0cda289e56b43bd08a020b976de62ed2bbf04b1e961fb775093c0830
contains two informational SetValue same-value40 countercases: false records only
the geometry endpoint; true records native callback23/tag4/value40/committed40,
internal32, then geometry endpoint. These establish the executed core predicate
within their declared record-only callback/geometry/ownership endpoints. The public
39255B30 adapter/392557E0 typed float4+bool2 reader/39254F00 receiver path is SOURCE
ONLY for the actual caller's number+explicitbool. These counters are not new5PASS,
full FH/Lua classification, native ownership or geometry-success evidence. The new
five-case native batch remains HELD/NOT RUN. The separate Root-reported AG20 provider
rejection after its checkpoint remains preserved and is not retried or rerouted.
No new native/emulator/IDA work accompanies this authoring.

Actual pre-edit host-only R4 RED was independently confirmed by Root. One focused
Cargo test compiled successfully in38.15s, ran in2.26s, and exited actual101 after
41.08s (PID12472 absent, compiler released before analysis). The full original Options
constructor and unchanged original Sound Init passed live-class/function-identity,
range, provider and script/handler admissions. All five controls began at10 from
original SetValue(10,false), with actual option1 and extents356x26; values/options
remained10/1. Actual trace was only Init35,36,37,39,40; actual provider records were[]
versus required[[35,1],[36,1],[37,1],[39,1],[40,1]]. The first behavior assertion at
slider_setvalue.rs81 failed; later strict handler/provider order, nested-false and
quality/checkbox exclusion assertions were UNREACHED, not passed. This proves skipped
used handler/data application, not audio/backend operation or a live wrong visual.
Root proof is ROOT_SETVALUE_R4_BEHAVIOR_RED_CONFIRMATION.json in its ignored checkpoint
folder. Exact old widget/test/fixture/runner/basis/log/process and Root proof were
archived before implementation under pre-implementation-r5/.

The complete original primary test's5132-byte prefix and fixture's8861-byte prefix
are preserved, including every strict assertion. Four newly authored own controls
cover true same/changed/unchanged clamp, false changed/same, hidden/disabled
programmatic calls, existing tick-delayed retirement/dead guards, and finite reentry
with committed values and nested false suppression. They are not executed yet.
The primary's debug observer leaves original constructor/Init/OnSliderChanged/
SetVolume/SetOptionItemValue/storage-provider functions installed unchanged. Earlier
static draft correction for absent optional AAUI.OptionChanged remains archived;
no synthetic audio/provider success callback was installed or relabeled a failed run.
The actual existing storage-provider entry/return is the observation boundary.

The new native_slider_setvalue example is SOURCE ONLY: default_plugins+DevPlugin,
Option<NonSendMut<UiRuntime>> readiness guard before consuming Local done, original
full Sound page at.85 with original widgets/art, and five actual provider-record
assertions/logging. It adds no technical probe overlay. Fresh defaults may look the
same before/after; this scene is a smoke/provider regression, not a visible fix or
pixel/native-font/FPS/scale-geometry parity proof. No build/copy/GPU/FPS result exists.
The proposed normal Cancel/close/reopen SAME-frame hypothesis remains DISPROVED:
original OnHide clears optionFrame and next Toggle constructs again. It is never
used as this change's live visual justification or fixture supplier.


A narrow sixth-path authoring exception was separately approved from Root SOURCE
inspection: only page_is_not_step_snapped_and_clamped_unchanged_still_notifies_once
in tests/slider_page.rs changes one Lua string. Its old held probe explicitly expected
SetValue(same,true) to notify0; the proven ordinary public contract makes that obsolete.
The replacement stores before, requires exactly1 callback with last==before and
GetValue()==before, then resets changes0 before all existing strict page pointer/
release/count1/value/last assertions. This is not >= weakening or a changed input
contract. Old17247-byte/e7ec1f5e... full source and Root approval were archived BEFORE
edit; raw prefix7606 and suffix8508 outside that function are exact, including the
prior accepted range callback observer. Only the one approved string differs.
This is a SOURCE-derived exception, NOT an executed protected failure; no new
protected run occurred. The independently confirmed primary SoundR4 RED is unchanged.

Current status: exact-six-path AUTHORING freeze, no post-edit Runtime/Cargo,
protected/full-suite checks, example build, independent R1 result, Clippy or GPU
validation yet. Root's previous integrated passes/play alias contain source6661,
not this uncompiled authoring. All future commands require separate resource grants;
peer's runtime window hold is respected. No production cleanup, foreign source,
manifest/dependency, child, index mutation, commit or push is part of this step.


### 2026-10-26 stopping checkpoint: bounded SetValue change, visual validation incomplete

The user requested a stopping point. The five Rust paths remain byte-identical
to the R6 source reviewed QUALIFIED CLEAR by independent R1. This append preserves
the entire preceding 659995-byte R1 document prefix; its earlier authoring/pending
statements are historical. This is not an overall GREEN or completed visual, native,
audio/backend, or main-client integration checkpoint. No further feature work,
compiler/GPU/native run, cleanup annotation, or source correction is queued.

The used contract remains finite f32 numeric plus an EXPLICIT Boolean public
Slider.SetValue on live finite ordered Slider state. True commits/clamps and
notifies even unchanged; false commits without notification. The existing input
helper and accepted range/page contracts remain unchanged. The original Sound
constructor/Init/handlers/SetVolume/storage provider remain the actual consumer.
Missing flags/defaults/coercion/nonfinite/invalid-state/signed-zero/native FH,
callback-visible thumb geometry/internal32 and broader slider behavior remain
qualified as previously documented. New native5 was NEVER RUN; the retained two
core countercases and SOURCE-only typed adapter are not full native Lua validation.
The normal same-frame Cancel/close/reopen hypothesis remains DISPROVED.

Actual CPU/review results on those frozen bytes:

- Independent R1: QUALIFIED CLEAR, no P1/P2/P3 in its bounded scope.
- Focused page10/range12/SetValue5: 27 passed, 0 failed, 0 ignored. The full original
  Sound Init reached the strict 30-row trace and ordered provider writes/returns
  for IDs35,36,37,39,40, each value1, while controls stayed10/options1.
- Selected library plus11 protected targets: 164 passed, 0 failed, 1 existing
  text::tests::font_metrics ignore. These overlap the full run; counts are not summed.
- Full --tests: actual Cargo OS0, 733 passed, 0 failed, 1 same font ignore across
  102 targets, 514.965283s. The ORIGINAL Python wrapper nevertheless exited OS1
  on immediate PID queryability, retains postflight_pins_passed=false and FAILED
  classification, and was never relabeled successful. Separate Root post-terminal
  verification established PID absence and full source/evidence guards; see
  ROOT_SETVALUE_R6_FINAL_TESTS_POSTTERMINAL_VERIFICATION.json (SHA8ca7d1f1...).
  Why the PID was queryable then is UNDETERMINED. No tests were rerun to repair it.
- Doctests: 2 passed, 0 failed/ignored, actual OS0 with R7 postguards.
- Exactly ONE final scoped Clippy: actual OS0, 142 warning emissions, 115 unique
  diagnostic-site records, 0 errors, 0 primary emissions in the six owned paths.
  Raw JSONL, separate stderr and complete sites are retained. The duplicate
  aa-client bin/example manifest warning is also retained. No baseline-clean
  claim, suppression/annotation cleanup or second lint was made.

The frozen example built OS0 and was copied as qa-native-slider-setvalue-r6.exe,
113753088 bytes, SHA25622f6d13f9629f4f2c51462eb0be66a2f52c2cd096268b9378928b03c775649c7.
Its copy-time NOT LAUNCHED status is historical. The ONE later GPU attempt hit the
60s limit: owned-child termination OS1, 60.672s, NO PNG, NO FPS lines and NO provider
receipt. The last log was UI ready33940ms/4681objects/startup0 script errors.
WARN0/ERROR0/panic0 is not a clean capture or completed scenario. Source ordering
places UI ready after UiSetup returns and before runtime insertion; generic
virtual-time1s/2s/6s gates do not measure actual wall-clock/frame progress. Actual
virtual elapsed, pause and render stages were not measured; capture cause remains
UNDETERMINED. No image/native-pixel/font/audio success is claimed and no retry or
cap increase was performed.

The separately authorized TEMP host diagnostic compiled, then reported ONE logical
case0 passed/1 failed: its owned60s worker timed out; Cargo101, total75.802s.
The full Options install returned in2.513s; the exact .85/Showtrue/SelectSound chunk
returned in56.227s. UISETUP_RETURNED4681/.85 and Tick0 BEFORE occurred near59.091s;
about0.9s nominal budget remained. No Tick0 AFTER, layout, four controlled250ms
ticks, observed Sound.Init or strict provider-report stage was reached. This does
NOT establish tick nontermination, an infinite Lua loop, a source API defect or
the original GPU cause. The outer wrapper separately failed its historical-doc
PID queryability postguard; reason is UNDETERMINED and no foreign process was
controlled. All four owned PIDs were independently absent before analysis.

Root explicitly decided to remove only the temporary diagnostic test before an
exact-six checkpoint. Its10606 bytes/SHA52b0962aec144e67b76839eb9f391e830b0387aae6204f222020f90cd9b052b4
matched its frozen and executed-failure copies; source/results/raw logs were
preserved first. It is not a feature test or seventh commit path. All old freezes,
failed wrappers/captures, raw results and native qualifications remain immutable
under data/extract/aa-ui/active-next-20261026-setvalue-r1/ and the Root review folder.
This checkpoint adds factual documentation only; no Rust code changed, no current
main binary/play alias was built or validated for R6, and no integration or overall
parity completion is implied. Root owns the exclusive Git/worktree stopping lane.

## Browser (wasm32-unknown-unknown)

* Native cursor extraction from `archeage.exe` (`cursor::sync`) is skipped: the page keeps its own cursor.
* The clipboard is only the in-process buffer (and the `AAUI.GetClipboard`/`SetClipboard` hooks); there is no OS clipboard off Windows.
* `mlua::Integer` is 32 bits on wasm32, so numbers passed to Lua events go through `lua_int` as `Value::Number` (Lua 5.1 numbers are doubles anyway).
* `ui_texts` is read from the game database through the in-memory SQLite VFS (see aa-anim's Browser note).
