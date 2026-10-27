# aa-lua: the client's UI Lua in Rust

Runs the original ArcheAge 1.2 UI scripts (`.alb` in `game_pak` under `scriptsbin/`) in a Lua 5.1
state (`mlua`, vendored 5.1.5), in the order the client loads them.

## API

```rust
pub fn new_lua() -> mlua::Result<mlua::Lua>;                  // 5.1 state: all std libs + Script.ReloadScript/LoadScript/UnloadScript
pub fn load_alb(lua: &Lua, pak_path: &str) -> anyhow::Result<Function>;  // convert + compile one chunk (.alb or .lua name)
pub fn toc_files(toc_path: &str) -> anyhow::Result<Vec<String>>;         // resolved pak paths, load order
pub fn run_toc(lua: &Lua, toc_path: &str) -> anyhow::Result<()>;         // run all, keep going past failures, Err lists them

// more
pub fn load_alb_bytes(lua, bytes, name) / convert_alb(bytes, name)       // bytes in memory
pub fn run_alb(lua, pak_path)                                            // load + call
pub fn run_toc_collect(lua, toc) -> Vec<(path, error)> / run_files(lua, &[paths])
pub fn run_stage(lua, Stage::WorldSelect)                                // a whole screen in client order
pub fn alb_path("x.lua") -> "x.alb";  script_path("SCRIPTS/x2ui/a.lua") -> "scriptsbin/x2ui/a.alb"
pub enum Stage { Login, WorldSelect, CharacterSelect, CharacterCreate, World, Loading }
    .tocs() .units() .scripts()   stage::INIT_SCRIPT, BASE_CONTENTS, WORLD_ROOT_SCRIPTS
pub mod chunk   // parse/write/convert Lua 5.1 chunks, disassembler
pub mod recorder // API recorder (docs/ui-api.md, data/ui-api.json)
```

Typical use (aa-ui): `new_lua()`, install the native API as globals (`UIParent`, `X2*`, widget
userdata, constants), `run_alb(&lua, stage::INIT_SCRIPT)`, then `run_stage(&lua, Stage::Login)`.

`mlua::Error` is not `Send + Sync` here (mlua without its `send` feature), so convert with
`.map_err(|e| anyhow::anyhow!("{e}"))` when using `?` into `anyhow`.

CLI: `cargo run -p aa-lua --bin aa-lua -- dis <path> | toc <toc> | stage <name> | convert <prefix> <outdir> | run <toc>... | record`.

## `.alb` format

Stock `luac` 5.1 output for a 32-bit host whose `lua_Number` is `float`:

```
1b 4c 75 61  51  00  01  04   04      04          04         00
ESC "Lua"    5.1 fmt LE  int  size_t  Instruction lua_Number integral=0
```

A 64-bit host Lua expects `size_t` 8 and `double`, so `chunk::convert` parses the chunk and writes
it back with 8-byte `size_t` and `double` constants (each `float` constant is widened through its
shortest decimal form, so `0.7f` becomes `0.7`, not `0.699999988`). Everything else is unchanged:

* **Opcodes are the stock 5.1 set** (0 MOVE … 37 VARARG, same numbering and operand layout);
  verified by disassembly (`aa-lua dis`) and by unluac decompiling all 961 files cleanly.
* Chunks are stripped of the source name (`=?` in client errors) but keep line info and local
  names. `convert` records `@scriptsbin/...alb` as the source so errors name the file.
* Re-encoding a parsed chunk in the original layout reproduces every file byte for byte (test).
* `mlua` loads binary chunks in any state (`ChunkMode::Binary`), no `unsafe_new` needed for that;
  `new_lua` uses `unsafe_new_with(StdLib::ALL)` only to get the `debug` library the client has.

The client's numbers are 32-bit floats; ours are doubles. Integers above 2^24 and float rounding
can differ slightly from the real client.

## Load order (from `x2game.dll`)

Recovered from the unpacked `x2game.dll` (`<client12-unpacked>\x2game_unpacked.dll`,
`X2::GameClient::UIContentsManager`):

* `InitUi` (UI state created): `scripts/x2ui/baselib/locale_helper.lua`.
* `LoadBaseLib`: contents `logic`, `baselib`, `components`, `option` (each content = its `toc.g`).
* Login screens (`StartScreenLogin/WorldSelect/CharacterSelect/CharacterCreate`): LoadBaseLib, then
  `loginstage/<screen>`; character select also `secondpassword`. Each loginstage toc starts with
  `../baselib.lua` (= `loginstage/baselib.alb`, a different file from `baselib/baselib.alb`).
* World (`StartScreenWorld`): LoadBaseLib, `ui_management.lua`, `LoadAllContents` (every
  `scriptsbin/x2ui/<dir>` with a `toc.g`, in pak find order; we use alphabetical and skip the base
  contents; `loginstage` has no `toc.g`), then `npc_common, character_ability, faction, interaction,
  npc_quest_interaction, store_backpack, transfer, dynamic_action_bar, ucc, demo, npc_interaction,
  renew_item` (`.lua`). The `*_view.lua` files next to them are pulled in by
  `Script.ReloadScript`.
* Loading screen (`LoadGlobalUI`): `scriptsbin/globalui/loading/toc.g`.
* Not established: whether the Lua state is reset between screens (we assume a fresh state per
  screen), and the exact directory enumeration order in game.

`toc.g` resolution (`LoadContent`): `#` lines are comments; `SCRIPTS/x` = `<root>/x` with root
`scriptsbin/x2ui` (or `scriptsbin/globalui` for global UI); other lines are relative to the content
directory, `../` strips one directory; `.lua` -> `.alb` (CVar `lua_use_binary` = 1). A failing file
is logged (`cannot load %s file.`) and the next one runs.

**Locales:** every toc lists all locale files and the client runs them all; each one checks
`X2Locale:GetLocale() == "en_us"` (and sometimes `X2Util:GetGameProvider() == TRION`) itself, so
the native side just has to answer `en_us`.

Per-screen lists: `cargo run -p aa-lua --bin aa-lua -- stage world_select`.

## Decompiled sources

All 961 scripts decompiled with unluac (1.2.3.569) to `data/extract/aa-lua/decompiled/` (mirrors
`scriptsbin/`, `.alb` -> `.lua`, 0 failures). Regenerate:

```sh
# needs java; unluac from https://sourceforge.net/projects/unluac/ in data/extract/aa-lua/tools/unluac.jar
crates/aa-lua/scripts/decompile.sh            # [src_dir] [out_dir] [jar]
```

unluac reads the float-number header directly; no conversion step is needed.

## Native API extraction (`data/x2game.json`)

`scripts/re/x2game_api.py <x2game_unpacked.dll> crates/aa-lua/data/x2game.json` (Python + `pefile`)
extracts from the client DLL: the 71 native API table names (`ADDON`, `UI`, `X2Console` …
`X2Achievement`, by API id), the 626 UI event names by event id (`FireUIEvent` ids: 70
`ENTERED_LOGIN`, 477 `ENTERED_WORLD_SELECT`, 78 `ENTERED_WORLD`), and 2491 natively registered
script functions (`SUserFunctionDesc {name, params}`) with their parameter names
(`SelectCharacter(index, skipClientDriven)`), in 119 groups by registering bind class. The events
come from a static initializer (`push name; mov ecx, slot; call`), so their id is their index.

## API recorder (`docs/ui-api.md`, `data/ui-api.json`)

```sh
cargo run -p aa-lua --bin aa-lua -- record [out.json] [out.md]   # default data/ui-api.json, docs/ui-api.md
```

The recorder combines three sources:

1. **Static scan** of all 731 UI chunks (`recorder::static_scan`). It tracks registers to find:
   * global reads and writes;
   * `Global:Method(...)` and `Global.field` with argument counts;
   * every `self` method name;
   * field writes and method definitions;
   * string constants;
   * how each native return value is used (number, string, table with fields, boolean, truth
     test).
2. **Dynamic run** of each screen in client order (`recorder::record_stage`) in a fresh state.
   * Every global that the scripts do not define is a recording proxy (a table sharing one
     metatable).
   * Widgets are created by `CreateWidget`, `CreateChildWidget` and `CreateChildWidgetByType`; they
     are typed proxies whose method lists come from the reference capture.
   * Native constants and data tables get their real values from the reference.
   * Native returns are neutral values of the statically inferred kind (`0`, `""`, `false`), else
     a proxy.
   * After loading, every recorded handler (`SetHandler`, `OnEvent` per `RegisterEvent`) is called
     with fuzzed arguments, under an instruction budget.
   * The prelude is `src/recorder.lua`.
3. **Reference**: `reference/api/` (real-client captures by the `reference` agent) and
   `data/x2game.json`. Every recorded member is checked against them (`~` means native but unused,
   `!` means not native).

The output is ranked by call sites and grouped into:

* widget toolkit (28 widget types, 18 drawable/object types, 39 handlers);
* X2 game API (68 tables);
* CryEngine (`Script`, `Console`, `Movie`);
* other native functions and values (with their values);
* unresolved globals;
* events (all 626, with id, who registers them, and which login screen);
* bind groups.

§0 covers the login stage on one page: widget types, objects, the X2 members called on each login
screen (bold = actually called while running), and the events per screen.

**Results:**

| Screen | Scripts | With load errors | Handler calls |
|---|---|---|---|
| login, world select, character select, character create, loading | all | 0 | ~2400 |
| world | 606 | 10 | 7328 |

The 10 world errors come from native data we return neutral values for, such as a `maxHp` string
and an `sformat` nil.

**Load-order check:** after world select, the global Lua functions defined are exactly the real
client's 534 (from `reference/api/x2_globals.json`), with 0 missing and 0 extra.

### Native behaviour the scripts rely on

The scripts depend on the following native behaviour. aa-ui must implement it.

* **`parent:CreateChildWidget(type, name, index, true)`** also stores the child on the parent:
  `parent[name]` when index is 0, `parent[name][index]` otherwise. Scripts then use
  `window.titleBar.closeButton` without ever assigning it.
* **Native style fields on widgets:** `style`, `titleStyle`, `itemStyle`, `extraStyle`,
  `childStyle`, `prefixStyle`, `cooltime_style`. These are textstyle objects.
* **`tab:AddSimpleTab` / `AddTab`** create `tab.window[i]`, `tab.selectedButton[i]` and
  `tab.unselectedButton[i]`.
* **`listctrl:InsertColumn`** creates `listctrl.column[i]`.
* **`listctrl:InsertRows(n)`** creates `listctrl.items[i].subItems[col]`.
* **`ADDON:RegisterContentWidget(id, w)`** and **`ADDON:GetContent(id)`**: `GetContent` returns the
  stored widget, or nil if none was registered. Returning a dummy at login makes
  `option/interface_option` fail where the real client does not.
* **Events:**
  * `RegistUIEvent(window, table)` is script code; it calls `window:RegisterEvent(name)` per key.
  * `OnEvent` handlers are called as `(this, event, ...)`.
  * `UIParent:SetEventHandler("ENTERED_WORLD", fn)` is the other path; it takes the event name.
* **Number comparisons:** Lua 5.1 raises an error when comparing a number with a table, before it
  looks at any metamethod. Natives that return numbers must return real numbers.
