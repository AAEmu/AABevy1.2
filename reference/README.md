# reference/ : what the real ArcheAge 1.2 client shows

Captures taken from the **real** 1.2 client (`<client>`, r208022) running against a local AAEmu server. Everything is 1920x1080, windowed, DX10 back buffer, UI scale 1.0.
Use them to check the Bevy remake pixel for pixel and widget for widget.

## Index

| Path | What |
|---|---|
| `world_select/real.png`, `ui.json`, `camera.json` | Select Server, account with 0 characters (default state, nothing selected) |
| `world_select/real_selected.png`, `ui_selected.json` | same, server row selected, Connect enabled |
| `world_select/*_1_character*` | same with 1 character on the server (Characters column filled) |
| `world_select/real_1600x900.png`, `ui_1600x900.json` | the same screen at 1600x900, to see how layout reacts to the resolution |
| `character_select/0_characters/` | Select Character, 0 characters (4 "Create Character" slots, world map) |
| `character_select/1_character/` | Select Character, 1 character (`Refone`, Nuian male, Lv1 Battlerage) |
| `character_create/step1_<race>_<gender>/` | Select Race, 4 races x 2 genders (`nuian`, `elf`, `ferre`, `hariharan`; `male`, `female`) |
| `character_create/step2_<race>_<gender>/` | Appearance (hair / face tabs, 3D head close up) |
| `character_create/step3_<race>_<gender>/` | Select Skillset + name (3D training field, one scene per race) |
| `character_create/step3_nuian_male/*_ability_name*` | step 3 with Battlerage selected and name typed |
| `character_create/step3_nuian_male/*_confirm_dialog*` | the "Create Character / Finalize this character?" modal |
| `loading/real_01..04_*.png` | loading screen frames: fade from char select, 0 %, 48 %, 100 % (ship at sea, gauge, tip) |
| `loading/ui_third_state_all_widgets.json` | the loading screen widget tree (it lives in the *third* Lua state, see below) |
| `ingame/spawn/` | first frame in world (`real_first_frame.png`) and settled HUD (`real.png`) of the brand new character, ui.json, camera.json with player entity position |
| `ingame/character_info/` | Character Info window (C) with the 3D player model: window at (28,60) 430x519; the equipment slot columns are separate widgets at x=760 (left) and x=1118 (right), 42x42 slots, y=290..774, model centred near x=960 (`player_screen.json` = player screen position) |
| `ingame/options_keybinds/` | Options window (680x615, centred) on Key Settings - Game Keybinds; `row_titles.json` holds the row title strings (textbox `GetText()` is empty, see Quirks) with wrap info |
| `ingame/bag/` | Bag window (I / B) with the 4 starter items (4/50 slots), docked at the right |
| `ingame/worldmap_zone/` | World map (M) at zone level (Solzreed Peninsula / Desiree Peak, map size 3), window centred x 378..1545, y 222..858 |
| `ingame/worldmap_continent/`, `worldmap_world/` | same window after one / two right-clicks on the map: Nuia continent, then the world map |
| `api/widget_methods.json` | per widget class: full method list (`metatable.__index`); drawables; text style |
| `api/x2_globals.json` | every global of the UI / System / third Lua state split into stdlib, native, `X2*`, Lua-script functions, data tables (login stage) |
| `api/x2_globals_ingame.json` | same, in world (native surface identical, +760 script-defined UI functions) |
| `api/ui_data.json`, `ui_data_ingame.json` | every global Lua **data** table of the UI state: `TEXTURE_PATH`, `FONT_PATH/SIZE/COLOR`, `BUTTON_*` styles, `locale` strings, `LOGIN_STAGE_TEXTURE_PATH`, ... plus all scalar globals |
| `tools/` | the capture tooling (Lua library + Python driver), see "Reproduce" |

Each screen folder has `real*.png`, `ui*.json`, `camera*.json`. Variant suffixes (`_selected`, `_1_character`, ...) apply to all three.

## Facts the other agents need

* **Resolution / scale.** The UI is laid out in real pixels of the back buffer; `UIParent:GetUIScale()` is 1.0 at both
  1600x900 and 1920x1080. Widget sizes do not scale (a 171x57 button is 171x57 at any resolution); anchors
  (`CENTER`, `BOTTOM`, ...) move things. Full screen backgrounds are `width x (width * 0.625)` (16:10) centred
  vertically: 1600x1000 at y=-50, 1920x1200 at y=-60. Coordinates in `ui.json` are absolute top-left pixels
  (`GetEffectiveOffset` / `GetEffectiveExtent`).
* **Stages** (`X2LoginCharacter:GetCurrentStage()`): 1 logo/login movie, 3 server select, 4 character select,
  5 create: race, 6 create: appearance, 7 create: skillset+name, 9 loading, 0 in world (or logo before login).
  With 0 characters, entering a world goes **straight to stage 5** (create); "Select Character" (the prev button at step 1)
  leads to stage 4.
* **The UI Lua state is rebuilt on every stage change** (all globals vanish, widgets are re-created from the toc).
  The loading screen is not in the UI state at all: it is in the third state (`-t third`, global `loading`).
* **Cameras** (CryEngine, Z up, degrees, `angles = (pitch, roll, yaw)`, FOV vertical-ish as reported by
  `GetViewCameraFov`, converted to degrees):

  | Screen | pos (x, y, z) | angles | fov |
  |---|---|---|---|
  | server select | 624.9, 472.8, 233.2 | 45, 0, -90 | 80 |
  | character select | 725.8, 568.0, 298.1 | -5.43, -1.27, -177.94 | 30 |
  | create step 1 nuian | 725.2, 625.5, 298.3 | -8.31, 0, -158 | 25 |
  | create step 1 elf | 724.9, 600.4, 298.4 | -8.31, 0, -154 | 25 |
  | create step 1 ferre | 727.4, 676.7, 298.3 | -6.31, 0, -164 | 25 |
  | create step 1 hariharan | 727.4, 649.9, 298.4 | -8.31, 0, -163 | 25 |
  | create step 2 (head close up) | ~ step 1 position, a few units forward, pitch -3..+2, roll 0.15 | | 20 |
  | create step 3 elf / ferre / hariharan / nuian | (595.6,543.2,172.4) / (601.3,751.3,172.6) / (615.5,962.9,172.6) / (621.4,407.4,166.0) | yaw 98.9 / 95.8 / 96.9 / 100.1 | 40 |
  | in game, spawn | 4315.5, 4121.9, 128.0 (client local) | 0, 0, 158.7 | 42.75 |

  Player in game: entity class `nuian_male`, client-local position (4314.0, 4118.1, 126.2) on the terrain,
  yaw 2.7705 rad, zone "Solzreed Peninsula". These are client-local coordinates (the seamless map origin moves), not server
  global coordinates. `camera.json` also holds a few CVars (`e_time_of_day`, `cl_fov`, `r_width`, ...).
* **Widget model.** A widget or drawable is a Lua table `{__this = userdata, <script fields>}`; its metatable is its own
  `__index`, one flat method table per class (see `api/widget_methods.json`). Widget classes: `emptywidget window button label
  textbox editbox editboxmultiline listbox listctrl slot statusbar slider tab checkbutton folder grid gametooltip webbrowser
  chatwindow cooldownactionbutton cooldowninventorybutton cooldownconstantbutton cooldownbutton circlediagram modelview message
  damagedisplay roadmap`. Drawables: `image color text ninepart threepart iconimage effect`; plus a text **style** object
  (`widget.style`). Create with `UIParent:CreateWidget(type, name, 'UIParent')` or `widget:CreateChildWidget(type, name, ...)`;
  drawables `w:CreateImageDrawable(texturePath, layer)` etc. (signatures in `widget_methods.json`).
  Top level windows return themselves from `GetParent()`.
* **Frame rate** in the capture client is 15-23 fps (shared GPU/CPU with other sessions), so animation timing in the
  captures is not representative.

### What `ui.json` contains and what it cannot

`ui.json` = `{meta, widgets[], hidden_named_widgets[], hidden_named_total}`. `widgets[]` lists every effectively visible
object the scripts still reference, sorted by creation order (`order` = the hex `GetId` counter):

* all: `kind` (`widget|drawable|style`), `class`, `paths` (shortest Lua path from `_G`, e.g.
  `characterSelectWindow.rightPanel`, `step1.content.raceBtn[2]`, or `UpdateList^chInfoWindow` = reached through a closure
  upvalue), `eoffset`/`eextent` (absolute rect), `offset`/`extent`, `visible`, `enabled`, `alpha`, `parent` (widget) or `owner`
  (drawable / style = the widget holding it), `text` (labels and buttons), `line_count`, `longest_line_width`, `text_height`
  (textboxes), `line_height` (styles), button skin drawable ids (`normalbackground`, ... = addresses of other entries).
* **Not readable through the client API** (there are no getters): texture path and texture coords of a drawable, colours, font
  file and size, anchors, UI layer, the widget **name** (the Lua path stands in for it) and the text of a `textbox`
  (only its metrics). For these use the screenshot, the data tables in `api/ui_data.json` (texture paths, font paths and
  sizes, colours, locale strings) and the original scripts, which are what produce them. Running the original `.alb`
  scripts in the Bevy runtime and comparing `eoffset`/`eextent`/`text` with these dumps is the intended check.
* Modal dialogs (`X2DialogManager`, e.g. the create-character confirmation) and other widgets that only native code
  references are **not** reachable and are missing from `ui.json`; they are in the PNGs only.
* `snapshot.lua` can also walk the Lua registry (`R.enumerate(nil, 10, true)`) which finds those, but the registry keeps widgets
  that were destroyed natively and calling a getter on one **crashes the client**. It is off by default.

## Reproduce

Prerequisites: the `bevy` lane servers are up (`Scripts/live-stack.sh status` from `<aaemu-src>`), the lane's
`aaemu-clientdriver.exe` exists (`<server-target>\live`), and the x86 driver DLL is built:

```powershell
$env:AADRIVER_OUT = '<server-target>\aadriver'
<aaemu-src>\rust\crates\aaemu-clientdriver\native\build.bat      # aadriver.dll + aainject.exe
```

All steps below use `tools/capture.py` (Python 3, Pillow, numpy; `python -I`):

```bash
cd reference
python -I tools/capture.py launch            # starts MY client (launcher ticket login, nothing typed), forces 1920x1080,
                                             # writes the pid to <server>\logs\ref-client.pid (later commands read it)
aaemu-clientdriver window --pid <pid> hide   # recommended: hidden window keeps rendering and cannot be minimised or hit by
                                             # another session's SendKeys; screenshots come from the DXGI back buffer
python -I tools/capture.py to-create         # waits for server select, enters the world, waits for stage 4/5
python -I tools/capture.py screen world_select [--tag selected] [--no-ui] [--camera]   # real[_tag].png + ui[_tag].json (+ camera.json)
python -I tools/capture.py create-sweep      # all races x genders x 3 steps into character_create/ (~20 s per step)
python -I tools/capture.py api               # api/widget_methods.json, x2_globals.json, ui_data.json
python -I tools/capture.py api --suffix _ingame --no-widgets      # in world
python -I tools/capture.py lua "<expr>"      # run Lua in the UI state;  luas = System state
python -I tools/capture.py click X Y         # real mouse click (client pixels) through the client_pointer MCP tool
python -I tools/capture.py settle            # wait until the picture stops changing
```

Close your client with `taskkill /PID <pid> /F` (only the pid you started; `live-stack.sh stop` would stop the servers).
`tools/lib.lua` (JSON, enumeration, state getters, class probes, call recorder), `tools/snapshot.lua`, `tools/globals.lua`,
`tools/data_tables.lua` are plain Lua you can feed to `aaemu-clientdriver lua -t ui -` yourself.

### Screen flow, exact steps (all through Lua in the UI state)

| Step | Lua | Wait |
|---|---|---|
| launch to server select | `aaemu-launcher ... ` (see `capture.py launch`) | pipe answers after ~30-50 s; stage 3 about 25 s later |
| select server + connect | `backgroundWindow.mainWindow:SelectByDataKey(1)` then `X2World:EnterWorld(1,0)` | ~12 s; then stage 5 (0 chars) or 4 |
| character select to create | click the slot with `capture.py click` (**not** `openBtn:OnClick`: that crashes the client) | |
| race / gender | `step1.content.raceBtn[i]:OnClick("LeftButton")` i=1..4 (nuian, elf, ferre, hariharan); `step1.content.rightPanel.genderBtn[j]:OnClick("LeftButton")` j=1 male, 2 female | 5-14 s for the background fade |
| next / previous step | `characterCreate:OnNextStep()` / `characterCreate:OnPrevStep()` (the buttons' handlers are not fields, `nextStepBtn:OnClick` is nil) | step 2 ~3-12 s, step 3 ~9-12 s (the 2D backdrop fades out slowly) |
| skillset, name | `step3.content.rightPanel.abilityBtn[1]:OnClick("LeftButton")`; `step3.content.nameWnd.characterName:SetText("Refone")` | |
| finish | `characterCreate:OnCompleteCurrentStep()` opens the confirm modal; click Confirm at (917,580) with `capture.py click 917 580` | character appears in `rs-game.log` ("Registered character name"), stage 4 |
| start game | `characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')` | stage 9 loading 11 s (warm) to 35 s (cold), then stage 0 |
| back to step 1 from step 3 | `characterCreate:OnPrevStep()` twice, 3.5 s apart | |

Loading frames: after pressing Start run `capture.py frames <dir> --seconds 50` (screenshots only, ~0.25 s apart, no Lua), then
pick the fade, 0 %, 48 % and 100 % frames (frame diffs show the screen is static apart from the gauge).
In-game: wait for `X2:IsEnteredWorld()`, then `capture.py screen ingame/spawn`.

## Surprises / quirks

* **`system.cfg` wins over the command line.** `+r_width 1920 +r_height 1080 +r_fullscreen 0` on the launcher is ignored
  because `Documents\ArcheAge\system.cfg` (shared by every client on this machine; here 1600x900 windowed) is applied later.
  What works: `System.SetCVar('r_width', 1920)` and `r_height` through the driver (System state) after the pipe is up; the window
  and the whole UI re-layout live. `capture.py launch` does this. `system.cfg` was **not** modified (checked byte for byte).
* **Windowed client gets minimised** (something on this shared desktop, probably another session) and then every screenshot fails
  with "no rendered frame in 10 s". Hide the window with the driver (`window --pid N hide`); to recover a minimised one use `ShowWindow(h, SW_RESTORE)`.
* **Lua calls during a stage switch can crash the client** (access violation inside CryScriptSystem; seen three times: polling Lua right after
  `startButton`, calling `openBtn:OnClick` on a create slot, and a snapshot during create step 2 that touched dead registry widgets). During
  transitions use screenshots only and wait 10+ s before the next Lua call.
* **Native exception from a widget type**: `UIParent:CreateWidget('worldmap', ...)` throws natively. `GetValue()/GetMinMaxValues()` need a
  type string; calling them bare only logs an error.
* **Idle disconnect**: a client left at server select survived ~12 minutes here, then exited by itself.
* **Hooking the widget classes to read textures/fonts/anchors does not survive**: the UI Lua state (and every class method
  table) is re-created on each stage change, and the scripts re-run and build their widgets in the same call, so there is no point
  at which an outside driver can install wrappers before construction. The recorder code is still in `lib.lua`
  (`install_hooks`, `capture.py watch`) and works for anything created after it is installed.
* `GetId()` of a widget is a creation counter in hex, not the name; drawables/styles return garbage.
* The shared server also holds characters of other sessions; the test character here is `Refone` (account `clienttest`, Nuian male, Battlerage).
* Capturing a PNG costs ~0.3 s (DXGI hook), a `ui.json` snapshot 1-8 s depending on the stage (in world: ~25k registry objects, ~500 shown).
* Screen backgrounds animate (ambient motion, idle animations, fades): two captures of the "same" screen differ by ~0.6 grey levels on average.
* **In-game windows** (`ingame/*`): open with the hotkey actions through the MCP `client_action` tool (`toggle_character`, `toggle_bag`, `toggle_worldmap`;
  `capture.mcp_call('client_action', {'pid': pid, 'action': 'toggle_character'})`) or real key presses. World map zoom: `capture.click(pid, 860, 540, 'right_click')`
  goes zone -> continent -> world. The Options window is opened with real clicks (gear at (1897,1055), then the **Options** entry at (1820,977), then the
  **Game Keybinds** list entry at (698,633)); calling `ToggleOptionFrame()` through the driver threw a native "exception while running the request" in one session
  (three of the option sub-frames failed with `attempt to call a (null) value` and a later `debug.getinfo` on a script function killed that client) while the same
  window opened normally by clicking in a fresh session. Close it with Cancel (never Apply, it saves the options).
* **`textbox` widgets return `''` from `GetText()`** (text length 0) although they show text; labels and buttons are fine. Texts of such rows come from the
  `locale` tables (`ingame/options_keybinds/row_titles.json`). `GetLineCount` / `GetLongestLineWidth` / `GetTextHeight` do work and are in `ui.json`.
