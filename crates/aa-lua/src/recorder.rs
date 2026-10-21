//! API recorder: which native names the UI scripts use.
//!
//! Two passes over the client's UI scripts:
//!
//! * **Static** ([`static_scan`]): walk the bytecode of every UI chunk and collect global reads and
//!   writes, `Global:Method` / `Global.field` uses with argument counts, `obj:Method` names, table
//!   field definitions and string constants. Complete (every code path) but untyped.
//! * **Dynamic** ([`record_stage`]): run each screen's scripts in the client's order in a state where
//!   every global no script defines resolves to a recording proxy (see `recorder.lua`), then call
//!   every handler/callback the scripts registered. Gives widget types, which methods are called on
//!   which widget type, argument types, string arguments (event names, anchors, layers) and the
//!   fields read from returned values.
//!
//! [`build_report`] merges both with the native function list and event table extracted from
//! `x2game.dll` (`data/x2game.json`) into `data/ui-api.json` and `docs/ui-api.md`.

#![allow(clippy::type_complexity, clippy::unnecessary_sort_by)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context, Result};
use mlua::{Lua, Value};
use serde_json::{Map, Value as Json, json};

use crate::chunk::{self, Constant, Insn, Proto};
use crate::stage::{self, Stage};

/// `mlua::Error` is not `Send + Sync`, so `?` cannot turn it into `anyhow::Error`.
trait LuaResultExt<T> {
    fn lua(self) -> Result<T>;
}
impl<T> LuaResultExt<T> for mlua::Result<T> {
    fn lua(self) -> Result<T> {
        self.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// The recorder prelude (Lua source).
pub const PRELUDE: &str = include_str!("recorder.lua");

/// Native API data extracted from `x2game.dll` (see `scripts/re/x2game_api.py`).
pub const X2GAME_JSON: &str = include_str!("../data/x2game.json");

/// Lua standard globals (never native UI API).
pub const LUA_STD: &[&str] = &[
    "_G", "_VERSION", "assert", "collectgarbage", "coroutine", "debug", "dofile", "error", "gcinfo", "getfenv",
    "getmetatable", "io", "ipairs", "load", "loadfile", "loadstring", "math", "module", "newproxy", "next", "os",
    "package", "pairs", "pcall", "print", "rawequal", "rawget", "rawset", "require", "select", "setfenv",
    "setmetatable", "string", "table", "tonumber", "tostring", "type", "unpack", "xpcall", "arg",
];

/// CryEngine script-system globals (`CScriptBind_*`), as opposed to the X2 game API.
pub const CRYENGINE_GLOBALS: &[&str] =
    &["Script", "System", "Sound", "Game", "CryAction", "Entity", "AI", "Physics", "Particle", "Movie", "Log", "_time", "_frametime"];

// ---------------------------------------------------------------------------------------------
// Static pass

/// Use counts of one name (call sites or reads), by file.
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub count: usize,
    pub files: BTreeMap<String, usize>,
    /// Argument count (excluding `self`) -> call sites, when the call has a fixed count.
    pub argc: BTreeMap<usize, usize>,
    /// Call sites (vs. plain reads).
    pub calls: usize,
}

impl Usage {
    fn hit(&mut self, file: &str) {
        self.count += 1;
        *self.files.entry(file.to_string()).or_default() += 1;
    }
    fn call(&mut self, argc: Option<usize>) {
        self.calls += 1;
        if let Some(n) = argc {
            *self.argc.entry(n).or_default() += 1;
        }
    }
}

#[derive(Debug, Default)]
pub struct StaticScan {
    pub files: Vec<String>,
    /// Names assigned with SETGLOBAL by some script.
    pub global_writes: BTreeSet<String>,
    /// Globals read (GETGLOBAL).
    pub global_reads: BTreeMap<String, Usage>,
    /// `(global, member)` read or called via `Global.member` / `Global:member()`.
    pub members: BTreeMap<(String, String), Usage>,
    /// Every `x:Name(...)` method name, whatever `x` is.
    pub methods: BTreeMap<String, Usage>,
    /// Constant table keys assigned (`t.name = v`, `{ name = v }`, `function t:name()`).
    pub field_writes: BTreeSet<String>,
    /// Constant keys assigned a function (`function t:name()`): script-defined methods.
    pub method_defs: BTreeSet<String>,
    /// String constants.
    pub strings: BTreeMap<String, Usage>,
    /// How call results are used, by `(owner, member, result index)`; owner is the global
    /// (`X2Unit`), `"_G"` for global functions or `"?"` for methods on other values (widgets).
    pub returns: BTreeMap<RetKey, RetUse>,
}

/// Every UI chunk in the pak (`scriptsbin/x2ui/**`, `scriptsbin/globalui/**`).
pub fn ui_files() -> Vec<String> {
    let mut v: Vec<String> = aa_pak::Pak::global()
        .list("scriptsbin/")
        .map(aa_pak::normalize)
        .map(|n| n.strip_prefix("game/").map(str::to_string).unwrap_or(n))
        .filter(|n| (n.starts_with("scriptsbin/x2ui/") || n.starts_with("scriptsbin/globalui/")) && n.ends_with(".alb"))
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn static_scan(files: &[String]) -> Result<StaticScan> {
    let pak = aa_pak::Pak::global();
    let mut s = StaticScan { files: files.to_vec(), ..Default::default() };
    for f in files {
        let c = chunk::parse(&pak.read(f)?).with_context(|| format!("parsing {f}"))?;
        c.main.walk(&mut |p| scan_proto(p, f, &mut s));
    }
    Ok(s)
}

fn kstr(p: &Proto, i: u32) -> Option<String> {
    match p.constants.get(i as usize) {
        Some(Constant::String(b)) => Some(String::from_utf8_lossy(b).into_owned()),
        _ => None,
    }
}

fn rk_str(p: &Proto, x: u32) -> Option<String> {
    (x & 0x100 != 0).then(|| kstr(p, x & 0xff)).flatten()
}

/// How the scripts use the value a native call returns (static evidence).
#[derive(Clone, Debug, Default)]
pub struct RetUse {
    /// "number" (arithmetic, `<`, numeric `for`, `== 1`), "string" (`..`, `== "x"`), "table"
    /// (indexed), "boolean" (`== true`), "truth" (`if x`/`not x`), "function" (called), "length".
    pub kinds: BTreeMap<&'static str, usize>,
    /// Constant field names read from the value (`info.name`, `info:Method()`).
    pub fields: BTreeMap<String, usize>,
}

/// Owner of a call result: `(global or "?", member, result index)`.
pub type RetKey = (String, String, u8);

#[derive(Clone, Default)]
struct Reg {
    /// Global (or `Global.field`) the register holds.
    glob: Option<String>,
    /// `(receiver global, name)` of a method/field about to be called.
    meth: Option<(Option<String>, String)>,
    closure: bool,
    /// The register holds result `idx` of a native call.
    ret: Option<RetKey>,
}

/// Record a use of register/RK `x` as `kind`.
fn use_as(r: &[Reg], s: &mut StaticScan, x: u32, kind: &'static str) {
    if x & 0x100 == 0
        && let Some(key) = &r[x as usize].ret
    {
        *s.returns.entry(key.clone()).or_default().kinds.entry(kind).or_default() += 1;
    }
}

/// Record that register `x` is indexed (with constant key `field`).
fn field_of(r: &[Reg], s: &mut StaticScan, x: u32, field: Option<String>) {
    if x & 0x100 == 0
        && let Some(key) = &r[x as usize].ret
    {
        let e = s.returns.entry(key.clone()).or_default();
        *e.kinds.entry("table").or_default() += 1;
        if let Some(f) = field {
            *e.fields.entry(f).or_default() += 1;
        }
    }
}

fn scan_proto(p: &Proto, file: &str, s: &mut StaticScan) {
    for k in &p.constants {
        if let Constant::String(b) = k
            && let Ok(t) = std::str::from_utf8(b)
            && t.len() < 64
        {
            s.strings.entry(t.to_string()).or_default().hit(file);
        }
    }
    let mut r: Vec<Reg> = vec![Reg::default(); 256];
    let kind_k = |x: u32| -> Option<&'static str> {
        if x & 0x100 == 0 {
            return None;
        }
        match p.constants.get((x & 0xff) as usize) {
            Some(Constant::Number(_)) => Some("number"),
            Some(Constant::String(_)) => Some("string"),
            Some(Constant::Bool(_)) => Some("boolean"),
            _ => None,
        }
    };
    for &raw in &p.code {
        let i = Insn(raw);
        let (a, b, c) = (i.a() as usize, i.b(), i.c());
        match i.op() {
            0 => r[a] = r[b as usize].clone(), // MOVE
            5 => {
                // GETGLOBAL
                r[a] = Reg::default();
                if let Some(n) = kstr(p, i.bx()) {
                    s.global_reads.entry(n.clone()).or_default().hit(file);
                    r[a].glob = Some(n);
                }
            }
            7 => {
                if let Some(n) = kstr(p, i.bx()) {
                    s.global_writes.insert(n);
                }
            }
            11 => {
                // SELF A B C: R(A+1) = R(B); R(A) = R(B)[RK(C)]
                let recv = r[b as usize].glob.clone();
                let name = rk_str(p, c);
                field_of(&r, s, b, name.clone());
                let self_reg = r[b as usize].clone();
                r[a] = Reg::default();
                if a + 1 < 256 {
                    r[a + 1] = self_reg;
                }
                if let Some(m) = name {
                    s.methods.entry(m.clone()).or_default().hit(file);
                    if let Some(g) = &recv {
                        s.members.entry((g.clone(), m.clone())).or_default().hit(file);
                    }
                    r[a].meth = Some((recv, m));
                }
            }
            6 => {
                // GETTABLE A B C
                let recv = r[b as usize].glob.clone();
                let key = rk_str(p, c);
                field_of(&r, s, b, key.clone());
                r[a] = Reg::default();
                if let (Some(g), Some(k)) = (recv, key) {
                    s.members.entry((g.clone(), k.clone())).or_default().hit(file);
                    r[a].meth = Some((Some(g.clone()), k.clone()));
                    r[a].glob = Some(format!("{g}.{k}"));
                }
            }
            9 => {
                // SETTABLE A B C: R(A)[RK(B)] = RK(C)
                field_of(&r, s, a as u32, None);
                if let Some(k) = rk_str(p, b) {
                    if c & 0x100 == 0 && r[c as usize].closure {
                        s.method_defs.insert(k.clone());
                    }
                    s.field_writes.insert(k);
                }
            }
            12..=17 => {
                use_as(&r, s, b, "number");
                use_as(&r, s, c, "number");
                r[a] = Reg::default();
            }
            18 => {
                use_as(&r, s, b, "number");
                r[a] = Reg::default();
            }
            19 => {
                use_as(&r, s, b, "truth");
                r[a] = Reg::default();
            }
            20 => {
                use_as(&r, s, b, "length");
                r[a] = Reg::default();
            }
            21 => {
                for x in b..=c {
                    use_as(&r, s, x, "string");
                }
                r[a] = Reg::default();
            }
            23 => {
                // EQ A B C: kind from the constant on the other side
                for (x, other) in [(b, c), (c, b)] {
                    if let Some(k) = kind_k(other) {
                        use_as(&r, s, x, k);
                    }
                }
            }
            24 | 25 => {
                use_as(&r, s, b, "number");
                use_as(&r, s, c, "number");
            }
            26 => use_as(&r, s, a as u32, "truth"),
            27 => {
                use_as(&r, s, b, "truth");
                r[a] = Reg::default();
            }
            32 => {
                // FORPREP: init, limit, step
                for x in a..(a + 3).min(256) {
                    use_as(&r, s, x as u32, "number");
                }
            }
            36 => {
                r[a] = Reg::default();
                r[a].closure = true;
            }
            28 | 29 => {
                // CALL / TAILCALL A B C
                use_as(&r, s, a as u32, "function");
                let argc = (b != 0).then(|| b as usize - 1);
                let mut owner: Option<(String, String)> = None;
                if let Some((recv, m)) = r[a].meth.clone() {
                    // method call: first argument is self when it came from SELF
                    let is_self = recv.as_ref().is_none_or(|g| !g.contains('.'));
                    let n = argc.map(|n| if is_self { n.saturating_sub(1) } else { n });
                    if let Some(g) = &recv {
                        s.members.entry((g.clone(), m.clone())).or_default().call(n);
                    }
                    s.methods.entry(m.clone()).or_default().call(n);
                    owner = Some((recv.unwrap_or_else(|| "?".into()), m));
                } else if let Some(g) = r[a].glob.clone() {
                    s.global_reads.entry(g.clone()).or_default().call(argc);
                    owner = Some(("_G".into(), g));
                }
                for x in r.iter_mut().skip(a) {
                    *x = Reg::default();
                }
                // C-1 results land in R(A)...; C = 0 means "all" (track the first)
                if let Some((o, m)) = owner {
                    let n = if c == 0 { 1 } else { c as usize - 1 };
                    for k in 0..n.min(4) {
                        if a + k < 256 {
                            r[a + k].ret = Some((o.clone(), m.clone(), k as u8));
                        }
                    }
                }
            }
            // no register written
            8 | 22 | 30 | 34 | 35 => {}
            3 => {
                for x in r.iter_mut().take((b as usize).min(255) + 1).skip(a) {
                    *x = Reg::default();
                }
            }
            _ => r[a] = Reg::default(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Dynamic pass

/// One stage's recording (JSON straight from the Lua side) plus load errors.
#[derive(Debug, Default)]
pub struct StageRecording {
    pub stage: String,
    pub scripts: Vec<String>,
    pub rec: Json,
    pub load_errors: Vec<(String, String)>,
    pub handler_calls: usize,
    pub handler_failures: usize,
    /// Global script functions defined after loading (before handlers ran).
    pub defined_functions: BTreeSet<String>,
}

/// Instructions one script may run before it is stopped.
pub const SCRIPT_BUDGET: u32 = 20_000_000;
/// Instructions one handler call may run.
pub const HANDLER_BUDGET: u32 = 500_000;
/// Handler calls per stage.
pub const MAX_HANDLER_CALLS: u32 = 30_000;

fn set_of<'a>(lua: &Lua, names: impl IntoIterator<Item = &'a String>) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    for n in names {
        t.set(n.as_str(), true)?;
    }
    Ok(t)
}

/// The value kind a call result is mostly used as: "number", "string" or "boolean" (None for
/// tables/objects or no evidence).
pub fn value_kind(u: &RetUse) -> Option<&'static str> {
    let n = |k: &str| u.kinds.get(k).copied().unwrap_or(0);
    let (num, st, bo, tb) = (n("number"), n("string"), n("boolean"), n("table"));
    if num > 0 && num >= st && num >= tb {
        Some("number")
    } else if st > 0 && st >= tb {
        Some("string")
    } else if bo > 0 && tb == 0 {
        Some("boolean")
    } else {
        None
    }
}

/// `number`, `string`, `table{a,b}` … for a result's uses.
pub fn describe_ret(u: &RetUse) -> String {
    if let Some(k) = value_kind(u) {
        return k.to_string();
    }
    if u.kinds.contains_key("table") {
        let mut f: Vec<(&String, &usize)> = u.fields.iter().collect();
        f.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let names: Vec<&str> = f.iter().take(12).map(|(k, _)| k.as_str()).collect();
        return if names.is_empty() { "table".into() } else { format!("table{{{}}}", names.join(",")) };
    }
    if u.kinds.contains_key("function") {
        return "function".into();
    }
    if u.kinds.contains_key("truth") {
        return "truthy".into();
    }
    String::new()
}

/// What the real client's UI state contains, from the `reference` captures
/// (`reference/api/x2_globals.json`, `widget_methods.json`, `ui_data.json`, taken at world select).
#[derive(Debug, Default)]
pub struct Reference {
    /// Global -> value for plain numbers/strings (`ALIGN_CENTER` = 4, …).
    pub values: BTreeMap<String, Json>,
    /// Global -> contents for data tables (`FONT_COLOR`, …).
    pub tables: BTreeMap<String, Json>,
    /// Class -> native method names: X2 tables, `UIParent`/`ADDON`/`Sound`/…, widget types,
    /// drawables (recorder names like `ImageDrawable`) and `style` (text style objects).
    pub class_methods: BTreeMap<String, BTreeSet<String>>,
    /// Global script functions defined at capture time (to check the load order).
    pub lua_functions: BTreeSet<String>,
    /// Widget methods shared by every widget type.
    pub common_widget_methods: BTreeSet<String>,
    /// Plain native global functions (`band`, `bor`, ...).
    pub native_functions: BTreeSet<String>,
    /// Widget type names.
    pub widget_types: BTreeSet<String>,
}

impl Reference {
    /// Does the real client's UI state have this global (at world select)?
    pub fn has_global(&self, n: &str) -> bool {
        self.class_methods.contains_key(n) || self.values.contains_key(n) || self.tables.contains_key(n) || self.native_functions.contains(n)
    }
}

fn str_set(v: &Json) -> BTreeSet<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}

/// Load the reference captures from `dir` (normally `<workspace>/reference/api`), if present.
pub fn load_reference(dir: &std::path::Path) -> Option<Reference> {
    let read = |f: &str| -> Option<Json> { serde_json::from_slice(&std::fs::read(dir.join(f)).ok()?).ok() };
    let globals = read("x2_globals.json")?;
    let widgets = read("widget_methods.json").unwrap_or(Json::Null);
    let data = read("ui_data.json").unwrap_or(Json::Null);
    let ui = &globals["ui_state"];
    let mut r = Reference::default();
    for (k, v) in ui["other"].as_object().into_iter().flatten() {
        let Some(s) = v.as_str() else { continue };
        if let Some(n) = s.strip_prefix("number:") {
            if let Ok(x) = n.parse::<f64>() {
                let v = if x.fract() == 0.0 && x.abs() < 9e15 { json!(x as i64) } else { json!(x) };
                r.values.insert(k.clone(), v);
            }
        } else if let Some(t) = s.strip_prefix("string:") {
            r.values.insert(k.clone(), json!(t));
        }
    }
    for (k, v) in data.as_object().into_iter().flatten() {
        r.tables.insert(k.clone(), v.clone());
    }
    for section in ["x2", "native_tables"] {
        for (k, v) in ui[section].as_object().into_iter().flatten() {
            r.class_methods.insert(k.clone(), str_set(&v["native_functions"]));
        }
    }
    for (k, v) in widgets["widget_types"].as_object().into_iter().flatten() {
        r.class_methods.insert(k.clone(), str_set(&v["methods"]));
    }
    let drawable = |k: &str| match k {
        "color" => "ColorDrawable",
        "effect" => "EffectDrawable",
        "iconimage" => "IconImageDrawable",
        "image" => "ImageDrawable",
        "ninepart" => "NinePartDrawable",
        "text" => "TextDrawable",
        "threepart" => "ThreePartDrawable",
        _ => "",
    };
    for (k, v) in widgets["drawable_types"].as_object().into_iter().flatten() {
        let name = drawable(k);
        if !name.is_empty() {
            r.class_methods.insert(name.to_string(), str_set(&v["methods"]));
        }
    }
    let style = str_set(&widgets["textstyle"]["methods"]);
    if !style.is_empty() {
        r.class_methods.insert("style".into(), style.clone());
        r.class_methods.insert("titleStyle".into(), style);
    }
    r.lua_functions = str_set(&ui["lua_functions"]);
    r.common_widget_methods = str_set(&widgets["methods_common_to_all_widget_types"]);
    r.native_functions = str_set(&ui["native_functions"]);
    r.widget_types = widgets["widget_types"].as_object().into_iter().flatten().map(|(k, _)| k.clone()).collect();
    Some(r)
}

/// The workspace's `reference/api` directory.
pub fn reference_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/api")
}

fn json_to_lua(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        Json::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                // the capture writes integer keys as strings
                match k.parse::<i64>() {
                    Ok(i) => t.set(i, json_to_lua(lua, x)?)?,
                    Err(_) => t.set(k.as_str(), json_to_lua(lua, x)?)?,
                }
            }
            Value::Table(t)
        }
    })
}

/// Run one stage under the recorder.
pub fn record_stage(
    stage: Stage,
    scan: &StaticScan,
    native_names: &BTreeSet<String>,
    reference: Option<&Reference>,
) -> Result<StageRecording> {
    let lua = crate::new_lua().lua()?;
    let g = lua.globals();
    g.set("__REC_SCRIPT_GLOBALS", set_of(&lua, &scan.global_writes).lua()?).lua()?;
    g.set("__REC_SCRIPT_FIELDS", set_of(&lua, &scan.field_writes).lua()?).lua()?;
    g.set("__REC_NATIVE_NAMES", set_of(&lua, native_names).lua()?).lua()?;
    if let Some(r) = reference {
        // Real values for native constants and data tables no script defines.
        let values = lua.create_table().lua()?;
        for (k, v) in r.values.iter().chain(r.tables.iter()) {
            if !scan.global_writes.contains(k) {
                values.set(k.as_str(), json_to_lua(&lua, v).lua()?).lua()?;
            }
        }
        g.set("__REC_VALUES", values).lua()?;
        let cm = lua.create_table().lua()?;
        for (class, methods) in &r.class_methods {
            cm.set(class.as_str(), set_of(&lua, methods).lua()?).lua()?;
        }
        g.set("__REC_CLASS_METHODS", cm).lua()?;
    }
    // Return kinds inferred statically: "Owner:Member" -> { kind of result 1, kind of result 2, ... }
    let ret_by_key = lua.create_table().lua()?;
    let ret_by_method = lua.create_table().lua()?;
    for ((owner, member, idx), u) in &scan.returns {
        let Some(kind) = value_kind(u) else { continue };
        let (t, key) = if owner == "?" { (&ret_by_method, member.clone()) } else { (&ret_by_key, format!("{owner}:{member}")) };
        let list: mlua::Table = match t.get::<Option<mlua::Table>>(key.as_str()).lua()? {
            Some(l) => l,
            None => {
                let l = lua.create_table().lua()?;
                t.set(key.as_str(), &l).lua()?;
                l
            }
        };
        list.set(*idx as i64 + 1, kind).lua()?;
    }
    g.set("__REC_RET", ret_by_key).lua()?;
    g.set("__REC_RET_M", ret_by_method).lua()?;
    lua.load(PRELUDE).set_name("@recorder.lua").exec().map_err(|e| anyhow::anyhow!("recorder prelude: {e}"))?;
    lua.load(
        r#"function __REC_RUN(f, budget)
             debug.sethook(function() error("recorder: instruction budget exhausted", 0) end, "", budget)
             local ok, err = pcall(f)
             debug.sethook()
             if not ok then return tostring(err) end
           end"#,
    )
    .exec().lua()?;
    let run: mlua::Function = g.get("__REC_RUN").lua()?;
    let ctx: mlua::Table = g.get("__REC_CTX").lua()?;
    ctx.set("stage", stage.name()).lua()?;

    let mut scripts = vec![stage::INIT_SCRIPT.to_string()];
    scripts.extend(stage.scripts()?);
    let mut load_errors = Vec::new();
    for path in &scripts {
        ctx.set("file", path.as_str()).lua()?;
        ctx.set("phase", "load").lua()?;
        let f = match crate::load_alb(&lua, path) {
            Ok(f) => f,
            Err(e) => {
                load_errors.push((path.clone(), format!("{e:#}")));
                continue;
            }
        };
        if let Some(err) = run.call::<Option<String>>((f, SCRIPT_BUDGET)).lua()? {
            load_errors.push((path.clone(), err));
        }
    }
    let mut defined_functions = BTreeSet::new();
    for (k, v) in g.clone().pairs::<Value, Value>().flatten() {
        if let (Value::String(k), Value::Function(_)) = (k, v) {
            let k = k.to_string_lossy();
            if !k.starts_with("__REC") && !LUA_STD.contains(&k.as_str()) {
                defined_functions.insert(k);
            }
        }
    }
    let fuzz: mlua::Function = g.get("__REC_FUZZ").lua()?;
    let (handler_calls, handler_failures, _): (usize, usize, usize) = fuzz.call((HANDLER_BUDGET, MAX_HANDLER_CALLS)).lua()?;
    let rec: mlua::Table = g.get("__REC").lua()?;
    let mut out = Map::new();
    for key in ["members", "handlers", "events", "errors", "hints", "widgets"] {
        out.insert(key.to_string(), to_json(&rec.get::<Value>(key).lua()?, 0));
    }
    Ok(StageRecording {
        stage: stage.name().to_string(),
        scripts,
        rec: Json::Object(out),
        load_errors,
        handler_calls,
        handler_failures,
        defined_functions,
    })
}

/// Lua value -> JSON (tables with keys 1..n become arrays; functions and proxies are dropped).
fn to_json(v: &Value, depth: usize) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => json!(i),
        Value::Number(n) => json!(n),
        Value::String(s) => Json::String(s.to_string_lossy()),
        Value::Table(t) if depth < 8 => {
            let len = t.raw_len();
            let mut pairs: Vec<(Value, Value)> = t.clone().pairs::<Value, Value>().flatten().collect();
            if len > 0 && pairs.len() > len {
                // mixed: list part plus counts (the `strs` tables): keep as object
            } else if len > 0 && pairs.len() == len {
                pairs.sort_by_key(|(k, _)| match k {
                    Value::Integer(i) => i64::from(*i),
                    Value::Number(n) => *n as i64,
                    _ => 0,
                });
                return Json::Array(pairs.iter().map(|(_, v)| to_json(v, depth + 1)).collect());
            }
            let mut m = Map::new();
            for (k, v) in pairs {
                let key = match &k {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if matches!(v, Value::Function(_) | Value::UserData(_) | Value::Thread(_)) {
                    continue;
                }
                m.insert(key, to_json(&v, depth + 1));
            }
            Json::Object(m)
        }
        _ => Json::Null,
    }
}

// ---------------------------------------------------------------------------------------------
// Merge + report

/// Dynamic member record merged over stages.
#[derive(Clone, Debug, Default)]
struct Member {
    uses: u64,
    calls: u64,
    argc: BTreeMap<String, u64>,
    sigs: BTreeMap<String, u64>,
    strs: BTreeMap<usize, BTreeMap<String, u64>>,
    stages: BTreeSet<String>,
    files: BTreeMap<String, u64>,
    phases: BTreeSet<String>,
}

fn num(v: &Json) -> u64 {
    v.as_f64().unwrap_or(0.0) as u64
}

fn add_counts(dst: &mut BTreeMap<String, u64>, src: &Json) {
    if let Some(o) = src.as_object() {
        for (k, v) in o {
            *dst.entry(k.clone()).or_default() += num(v);
        }
    }
}

fn merge_members(recs: &[StageRecording]) -> BTreeMap<(String, String), Member> {
    let mut out: BTreeMap<(String, String), Member> = BTreeMap::new();
    for r in recs {
        let Some(members) = r.rec["members"].as_object() else { continue };
        for m in members.values() {
            let class = m["class"].as_str().unwrap_or("?").to_string();
            let member = m["member"].as_str().unwrap_or("?").to_string();
            let e = out.entry((class, member)).or_default();
            e.uses += num(&m["uses"]);
            e.calls += num(&m["calls"]);
            add_counts(&mut e.argc, &m["argc"]);
            add_counts(&mut e.sigs, &m["sigs"]);
            add_counts(&mut e.files, &m["files"]);
            for key in ["stages", "phases"] {
                if let Some(o) = m[key].as_object() {
                    let set = if key == "stages" { &mut e.stages } else { &mut e.phases };
                    set.extend(o.keys().cloned());
                }
            }
            // strs: { "1": { "<value>": count, "1": "<value>", ... } }  (list part + counts)
            if let Some(o) = m["strs"].as_object() {
                for (pos, vals) in o {
                    let pos: usize = pos.parse().unwrap_or(0);
                    if let Some(vals) = vals.as_object() {
                        let slot = e.strs.entry(pos).or_default();
                        for (k, v) in vals {
                            if v.is_number() && k.parse::<usize>().is_err() {
                                *slot.entry(k.clone()).or_default() += num(v);
                            }
                        }
                    }
                }
            } else if let Some(a) = m["strs"].as_array() {
                for (i, vals) in a.iter().enumerate() {
                    if let Some(vals) = vals.as_object() {
                        let slot = e.strs.entry(i + 1).or_default();
                        for (k, v) in vals {
                            if v.is_number() && k.parse::<usize>().is_err() {
                                *slot.entry(k.clone()).or_default() += num(v);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn top(m: &BTreeMap<String, u64>, n: usize) -> Vec<(&String, &u64)> {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    v.truncate(n);
    v
}

fn is_login(stages: &BTreeSet<String>) -> bool {
    Stage::ALL.iter().any(|s| s.is_login_stage() && stages.contains(s.name()))
}

fn is_const_name(n: &str) -> bool {
    n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && n.chars().any(|c| c.is_ascii_uppercase())
}

/// Everything needed for the report.
pub struct Inputs<'a> {
    pub scan: &'a StaticScan,
    pub recs: &'a [StageRecording],
    pub x2game: &'a Json,
    /// Real-client capture, if available (`reference/api`).
    pub reference: Option<&'a Reference>,
}

/// Build `(ui-api.json, ui-api.md)`.
pub fn build_report(inp: &Inputs) -> Result<(Json, String)> {
    let scan = inp.scan;
    let members = merge_members(inp.recs);
    let api_globals: BTreeSet<String> =
        inp.x2game["api_globals"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    let event_ids: Vec<String> =
        inp.x2game["events"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    let groups: Vec<(String, Vec<(String, Option<String>)>)> = inp.x2game["bind_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|g| {
            let f = g["functions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| (p[0].as_str().unwrap_or("").to_string(), p[1].as_str().map(String::from)))
                .collect();
            (g["register_fn"].as_str().unwrap_or("").to_string(), f)
        })
        .collect();
    let native_names: BTreeSet<String> = groups.iter().flat_map(|(_, f)| f.iter().map(|(n, _)| n.clone())).collect();

    // Files of the login stage (statically): every script the four login screens run.
    let mut login_files: BTreeSet<String> = BTreeSet::new();
    login_files.insert(stage::INIT_SCRIPT.to_string());
    for s in Stage::ALL.iter().filter(|s| s.is_login_stage()) {
        login_files.extend(s.scripts()?);
    }
    // plus what they pull in with Script.ReloadScript
    for ((class, member), m) in &members {
        if class == "Script" && member.ends_with("Script") && is_login(&m.stages) {
            for v in m.strs.get(&1).into_iter().flat_map(|s| s.keys()) {
                login_files.insert(crate::script_path(v));
            }
        }
    }
    let in_login = |u: &Usage| u.files.keys().any(|f| login_files.contains(f));

    // Native globals: read somewhere, never assigned by a UI script, not Lua std.
    let std: BTreeSet<&str> = LUA_STD.iter().copied().collect();
    let natives: Vec<(&String, &Usage)> = scan
        .global_reads
        .iter()
        .filter(|(n, _)| !scan.global_writes.contains(*n) && !std.contains(n.as_str()))
        .collect();

    // Classify.
    let mut game = Vec::new();
    let mut cry = Vec::new();
    let mut toolkit = Vec::new();
    let mut funcs = Vec::new();
    let mut values = Vec::new();
    let mut unresolved = Vec::new();
    let refr = inp.reference;
    for (n, u) in &natives {
        let has_members = scan.members.range((n.to_string(), String::new())..).take_while(|((g, _), _)| g == *n).next().is_some();
        if matches!(n.as_str(), "UIParent" | "UI") {
            toolkit.push((*n, *u));
        } else if n.starts_with("X2") || api_globals.contains(*n) {
            game.push((*n, *u));
        } else if CRYENGINE_GLOBALS.contains(&n.as_str()) || matches!(n.as_str(), "Console" | "Movie") {
            cry.push((*n, *u));
        } else if matches!(n.as_str(), "UIParent" | "UI") {
            toolkit.push((*n, *u));
        } else if refr.is_some_and(|r| !r.has_global(n)) {
            unresolved.push((*n, *u));
        } else if u.calls > 0 && !has_members {
            funcs.push((*n, *u));
        } else if has_members && !is_const_name(n) {
            game.push((*n, *u));
        } else {
            values.push((*n, *u));
        }
    }
    let by_count = |v: &mut Vec<(&String, &Usage)>| v.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
    for v in [&mut game, &mut cry, &mut toolkit, &mut funcs, &mut values, &mut unresolved] {
        by_count(v);
    }

    // Widget classes seen at run time.
    let mut widgets: BTreeMap<String, (u64, BTreeMap<String, u64>, BTreeSet<String>, bool)> = BTreeMap::new();
    for r in inp.recs {
        if let Some(o) = r.rec["widgets"].as_object() {
            for (t, w) in o {
                let e = widgets.entry(t.clone()).or_default();
                e.0 += num(&w["count"]);
                add_counts(&mut e.1, &w["by"]);
                if let Some(st) = w["stages"].as_object() {
                    e.2.extend(st.keys().cloned());
                }
                e.3 |= w["object"].as_bool().unwrap_or(false);
            }
        }
    }
    // class -> members (dynamic), for widget-ish classes: widget types, drawables, their field objects
    let mut class_members: BTreeMap<String, Vec<(&String, &Member)>> = BTreeMap::new();
    for ((class, member), m) in &members {
        class_members.entry(class.clone()).or_default().push((member, m));
    }
    let widget_class = |c: &str| -> bool {
        let base = c.split('.').next().unwrap_or(c);
        widgets.contains_key(base) || base == "UIParent"
    };

    // Owner of each native bind group: with the real-client capture, the class whose native
    // method list contains most of the group (`widget` = the methods every widget type has);
    // otherwise the class the scripts call most of its names on.
    let mut ref_classes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some(r) = refr {
        for (c, ms) in &r.class_methods {
            let specific: BTreeSet<String> =
                if r.widget_types.contains(c) { ms.difference(&r.common_widget_methods).cloned().collect() } else { ms.clone() };
            ref_classes.insert(c.clone(), specific);
        }
        ref_classes.insert("widget".into(), r.common_widget_methods.clone());
        ref_classes.remove("UI"); // same list as UIParent
        ref_classes.remove("titleStyle");
        ref_classes.remove("X2"); // same list as ADDON
    }
    let mut owners: Vec<(String, Option<String>, usize, Vec<(String, usize)>)> = Vec::new();
    for (addr, funcs_) in &groups {
        let names: BTreeSet<&str> = funcs_.iter().map(|(n, _)| n.as_str()).collect();
        let mut score: BTreeMap<String, usize> = BTreeMap::new();
        if !ref_classes.is_empty() {
            for (c, ms) in &ref_classes {
                let hits = names.iter().filter(|n| ms.contains(**n)).count();
                if hits > 0 {
                    score.insert(c.clone(), hits);
                }
            }
            let mut ranked: Vec<(String, usize)> = score.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let best = ranked.first().map(|(c, _)| c.clone());
            let best_n = ranked.first().map(|(_, n)| *n).unwrap_or(0);
            owners.push((addr.clone(), best, best_n, ranked.into_iter().take(4).collect()));
            continue;
        }
        for (g, m) in scan.members.keys() {
            if names.contains(m.as_str()) && !g.contains('.') && natives.iter().any(|(n, _)| *n == g) {
                *score.entry(g.clone()).or_default() += 1;
            }
        }
        for ((class, member), m) in &members {
            if m.calls > 0 && names.contains(member.as_str()) && widget_class(class) {
                // widget types share base classes: count the field-object class or "widget"
                let c = match class.split_once('.') {
                    Some((_, field)) => format!("*.{field}"),
                    None if widgets.get(class.as_str()).is_some_and(|w| w.3) => class.clone(),
                    None => "widget".to_string(),
                };
                *score.entry(c).or_default() += 1;
            }
        }
        let mut ranked: Vec<(String, usize)> = score.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let best = ranked.first().map(|(c, _)| c.clone());
        let best_n = ranked.first().map(|(_, n)| *n).unwrap_or(0);
        owners.push((addr.clone(), best, best_n, ranked.into_iter().take(4).collect()));
    }
    let params_of = |owner: &str, name: &str| -> Option<String> {
        let owner = if owner == "UI" { "UIParent" } else if owner == "X2" { "ADDON" } else { owner };
        groups.iter().zip(&owners).find_map(|((_, f), (_, o, _, _))| {
            if o.as_deref() == Some(owner) { f.iter().find(|(n, _)| n == name).and_then(|(_, p)| p.clone()) } else { None }
        })
    };
    let params_any = |name: &str| -> Vec<String> {
        let mut v: Vec<String> = groups
            .iter()
            .flat_map(|(_, f)| f.iter().filter(|(n, _)| n == name).map(|(_, p)| p.clone().unwrap_or_default()))
            .collect();
        v.sort();
        v.dedup();
        v
    };

    // ---------------- JSON
    let rets = |owner: &str, member: &str| -> Vec<String> {
        let mut v = Vec::new();
        for idx in 0..4u8 {
            match scan.returns.get(&(owner.to_string(), member.to_string(), idx)) {
                Some(u) => v.push(describe_ret(u)),
                None => break,
            }
        }
        while v.last().is_some_and(|s| s.is_empty()) {
            v.pop();
        }
        v
    };
    let usage_json = |u: &Usage| {
        json!({
            "sites": u.count,
            "calls": u.calls,
            "argc": u.argc.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
            "files": u.files.len(),
            "login": in_login(u),
        })
    };
    let member_json = |m: &Member| {
        json!({
            "runtime_uses": m.uses,
            "runtime_calls": m.calls,
            "argc": m.argc,
            "arg_types": top(&m.sigs, 8).into_iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
            "string_args": m.strs.iter().map(|(p, vals)| (p.to_string(), json!(top(vals, 25).into_iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()))).collect::<Map<_, _>>(),
            "stages": m.stages,
            "login": is_login(&m.stages),
            "handler_only": !m.phases.contains("load"),
        })
    };
    let global_json = |list: &[(&String, &Usage)]| -> Map<String, Json> {
        list.iter()
            .map(|(n, u)| {
                let mut o = usage_json(u);
                let mut mem = Map::new();
                for ((g, m), mu) in scan.members.range((n.to_string(), String::new())..).take_while(|((g, _), _)| g == *n) {
                    let _ = g;
                    let mut mo = usage_json(mu);
                    let r = rets(if n.as_str() == "UIParent" { "UIParent" } else { n.as_str() }, m);
                    if !r.is_empty() {
                        mo["returns"] = json!(r);
                    }
                    if let Some(p) = params_of(n, m) {
                        mo["params"] = json!(p);
                    }
                    if let Some(d) = members.get(&(n.to_string(), m.clone())) {
                        mo["runtime"] = member_json(d);
                    }
                    mem.insert(m.clone(), mo);
                }
                // natives of this table the scripts never use (real-client list, else bind group)
                if let Some(list) = refr.and_then(|r| r.class_methods.get(n.as_str())) {
                    for fname in list {
                        let p = params_of(n, fname).unwrap_or_default();
                        mem.entry(fname.clone()).or_insert_with(|| json!({"sites": 0, "params": p, "unused": true}));
                    }
                    for (m, mo) in mem.iter_mut() {
                        if !list.contains(m) && !mo["unused"].as_bool().unwrap_or(false) {
                            mo["not_native"] = json!(true);
                        }
                    }
                } else {
                    for ((_, f), (_, o2, _, _)) in groups.iter().zip(&owners) {
                        if o2.as_deref() == Some(n.as_str()) {
                            for (fname, p) in f {
                                mem.entry(fname.clone()).or_insert_with(|| json!({"sites": 0, "params": p.clone().unwrap_or_default(), "unused": true}));
                            }
                        }
                    }
                }
                if !mem.is_empty() {
                    o["members"] = Json::Object(mem);
                }
                let r = rets("_G", n);
                if !r.is_empty() {
                    o["returns"] = json!(r);
                }
                if let Some(d) = members.get(&("_G".to_string(), n.to_string())) {
                    o["runtime"] = member_json(d);
                }
                (n.to_string(), o)
            })
            .collect()
    };
    let mut widgets_json = Map::new();
    let mut objects_json = Map::new();
    for (t, (count, by, stages, object)) in &widgets {
        let mut ms = Map::new();
        let mut fields = Map::new();
        for (class, list) in class_members.range(t.clone()..) {
            let base = class.split('.').next().unwrap_or("");
            if base != t {
                if !class.starts_with(t.as_str()) {
                    break;
                }
                continue;
            }
            for (member, m) in list {
                let mut mj = member_json(m);
                let p = params_any(member);
                if !p.is_empty() {
                    mj["params"] = json!(p);
                }
                if class == t {
                    ms.insert(member.to_string(), mj);
                } else {
                    fields.insert(format!("{}.{member}", &class[t.len() + 1..]), mj);
                }
            }
        }
        let entry = json!({
            "created": count,
            "created_by": by,
            "stages": stages,
            "login": is_login(stages),
            "members": ms,
            "field_members": fields,
        });
        if *object { objects_json.insert(t.clone(), entry) } else { widgets_json.insert(t.clone(), entry) };
    }
    let mut handlers: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for r in inp.recs {
        if let Some(o) = r.rec["handlers"].as_object() {
            for (h, by) in o {
                add_counts(handlers.entry(h.clone()).or_default(), by);
            }
        }
    }
    let mut reg_events: BTreeMap<String, (u64, BTreeSet<String>, BTreeMap<String, u64>)> = BTreeMap::new();
    for r in inp.recs {
        if let Some(o) = r.rec["events"].as_object() {
            for (e, v) in o {
                let x = reg_events.entry(e.clone()).or_default();
                x.0 += num(&v["count"]);
                if let Some(st) = v["stages"].as_object() {
                    x.1.extend(st.keys().cloned());
                }
                add_counts(&mut x.2, &v["files"]);
            }
        }
    }
    let event_json: Vec<Json> = event_ids
        .iter()
        .enumerate()
        .map(|(id, name)| {
            let st = scan.strings.get(name);
            let re = reg_events.get(name);
            json!({
                "id": id,
                "name": name,
                "script_refs": st.map(|u| u.count).unwrap_or(0),
                "login": st.is_some_and(in_login) || re.is_some_and(|r| is_login(&r.1)),
                "registered": re.map(|r| r.0).unwrap_or(0),
                "registered_in": re.map(|r| r.1.iter().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            })
        })
        .collect();
    let unknown_events: Vec<&String> = reg_events.keys().filter(|e| !event_ids.contains(e)).collect();
    let mut errors: BTreeMap<String, u64> = BTreeMap::new();
    let mut load_errors = Vec::new();
    for r in inp.recs {
        for (f, e) in &r.load_errors {
            load_errors.push(json!({"stage": r.stage, "file": f, "error": e}));
            *errors.entry(short_err(e)).or_default() += 1;
        }
    }
    let script_methods: Vec<(&String, &Usage)> = {
        let mut v: Vec<_> = scan
            .methods
            .iter()
            .filter(|(m, _)| !scan.method_defs.contains(*m) || native_names.contains(*m))
            .filter(|(m, _)| native_names.contains(*m))
            .collect();
        v.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        v
    };
    let doc = json!({
        "about": "Native API used by the ArcheAge 1.2 UI scripts. Generated by `cargo run -p aa-lua --bin aa-lua -- record`; see docs/ui-api.md.",
        "load_order": Stage::ALL.iter().map(|s| (s.name().to_string(), json!(s.units()))).collect::<Map<_, _>>(),
        "stats": {
            "ui_files": scan.files.len(),
            "runs": inp.recs.iter().map(|r| json!({"stage": r.stage, "scripts": r.scripts.len(), "load_errors": r.load_errors.len(), "handler_calls": r.handler_calls, "handler_failures": r.handler_failures})).collect::<Vec<_>>(),
        },
        "toolkit": {
            "globals": global_json(&toolkit),
            "widgets": widgets_json,
            "objects": objects_json,
            "handlers": handlers,
            "methods_static": script_methods.iter().map(|(m, u)| (m.to_string(), { let mut o = usage_json(u); o["params"] = json!(params_any(m)); let r = rets("?", m); if !r.is_empty() { o["returns"] = json!(r); } o })).collect::<Map<_, _>>(),
        },
        "game": global_json(&game),
        "cryengine": global_json(&cry),
        "functions": global_json(&funcs),
        "values": values.iter().map(|(n, u)| {
            let mut o = usage_json(u);
            if let Some(v) = refr.and_then(|r| r.values.get(n.as_str()).or_else(|| r.tables.get(n.as_str()))) {
                o["value"] = v.clone();
            }
            (n.to_string(), o)
        }).collect::<Map<_, _>>(),
        "unresolved": unresolved.iter().map(|(n, u)| (n.to_string(), usage_json(u))).collect::<Map<_, _>>(),
        "events": event_json,
        "events_registered_unknown": unknown_events,
        "native_bind_groups": groups.iter().zip(&owners).map(|((addr, f), (_, o, n, ranked))| json!({
            "register_fn": addr,
            "owner_guess": o,
            "owner_hits": n,
            "candidates": ranked,
            "functions": f.iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "load_errors": load_errors,
    });

    // ---------------- Markdown
    let md = render_md(&doc, &errors);
    Ok((doc, md))
}

fn short_err(e: &str) -> String {
    // "running x: scriptsbin/...alb:123: attempt to ..." -> "attempt to ..."
    let tail = e.rsplit(": ").next().unwrap_or(e);
    let tail = tail.lines().next().unwrap_or(tail);
    tail.chars().take(90).collect()
}

fn argc_str(v: &Json) -> String {
    let Some(o) = v.as_object() else { return String::new() };
    let mut parts: Vec<(u64, &String)> = o.iter().map(|(k, c)| (num(c), k)).collect();
    parts.sort_by(|a, b| b.0.cmp(&a.0));
    parts.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>().join("/")
}

fn rets_str(v: &Json) -> String {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_str().filter(|s| !s.is_empty()).unwrap_or("?")).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

fn esc(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn render_md(doc: &Json, errors: &BTreeMap<String, u64>) -> String {
    let mut md = String::new();
    let w = &mut md;
    let _ = writeln!(w, "# ArcheAge 1.2 UI native API (what the original Lua UI expects)\n");
    let _ = writeln!(
        w,
        "Generated by `cargo run -p aa-lua --bin aa-lua -- record` (crate `aa-lua`). Machine-readable: \
         `data/ui-api.json`. Sources: a static scan of the bytecode of all {} UI scripts, a run of every screen's \
         scripts in the client's load order with a recording proxy behind every native name (plus one call of every \
         handler they register), and the native function and event tables extracted from `x2game.dll` \
         (`crates/aa-lua/data/x2game.json`).\n",
        doc["stats"]["ui_files"]
    );
    let _ = writeln!(
        w,
        "Columns: **sites** = call/read sites in the bytecode (all code paths), **argc** = argument counts seen \
         (most common first, `self` excluded), **params** = parameter names the client registers natively, \
         **types** = argument types seen at run time (`widget` = a widget or drawable, `native` = another native \
         value), **L** = needed by the login stage (world/character select/create). Names are ranked by sites.\n"
    );
    let _ = writeln!(w, "## Contents\n\n0. Login stage at a glance\n1. Load order\n2. Widget toolkit (widget types, drawables, handlers, methods)\n3. X2 game API\n4. CryEngine API\n5. Other native functions and values\n6. UI events\n7. Native bind groups (x2game.dll)\n8. Run errors\n9. Load-order check against the real client\n");

    // 0. Login summary
    let _ = writeln!(w, "## 0. Login stage at a glance\n\nWhat world select, character select and character create (and the login screen) need natively. Details in the sections below.\n");
    let mut wl: Vec<(&String, u64)> = doc["toolkit"]["widgets"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| v["login"].as_bool() == Some(true))
        .map(|(k, v)| (k, num(&v["created"])))
        .collect();
    wl.sort_by(|a, b| b.1.cmp(&a.1));
    let _ = writeln!(w, "**Widget types:** {}\n", wl.iter().map(|(k, _)| format!("`{k}`")).collect::<Vec<_>>().join(", "));
    let mut ol: Vec<&String> = doc["toolkit"]["objects"].as_object().into_iter().flatten().filter(|(_, v)| v["login"].as_bool() == Some(true)).map(|(k, _)| k).collect();
    ol.sort();
    let _ = writeln!(w, "**Drawables/objects:** {}\n", ol.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "));
    let _ = writeln!(
        w,
        "**Game API used by the login screens.** **Bold** = called while the login screens loaded or ran their handlers \
         in the recorder; plain = referenced somewhere in the scripts the login screens load (mostly shared baselib/option \
         code that is only reached in game).\n"
    );
    for sec in ["game", "cryengine", "toolkit"] {
        let list = if sec == "toolkit" { &doc["toolkit"]["globals"] } else { &doc[sec] };
        let mut gl: Vec<(&String, &Json)> = list.as_object().into_iter().flatten().collect();
        gl.sort_by(|a, b| a.0.cmp(b.0));
        for (g, v) in gl {
            let mut ms: Vec<(&String, bool)> = v["members"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, m)| m["login"].as_bool() == Some(true))
                .map(|(k, m)| (k, m["runtime"]["login"].as_bool() == Some(true) && num(&m["runtime"]["runtime_calls"]) > 0))
                .collect();
            if ms.is_empty() {
                continue;
            }
            ms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let names: Vec<String> = ms.iter().map(|(k, hot)| if *hot { format!("**{k}**") } else { k.to_string() }).collect();
            let _ = writeln!(w, "* `{g}`: {}", names.join(", "));
        }
    }
    let _ = writeln!(w, "\n**Events registered per screen** (id name):\n");
    for st in Stage::ALL.iter().filter(|s| s.is_login_stage()) {
        let evs: Vec<String> = doc["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["registered_in"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(st.name()))))
            .map(|e| format!("{} `{}`", e["id"], e["name"].as_str().unwrap_or("")))
            .collect();
        let _ = writeln!(w, "* **{}**: {}", st.name(), evs.join(", "));
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## 1. Load order\n");
    let _ = writeln!(
        w,
        "Every screen first runs `scriptsbin/x2ui/baselib/locale_helper.alb` once (UI state init), then these units \
         in order (a `toc.g` = its listed files; see `docs/aa-lua.md`). The four login screens each start from \
         `LoadBaseLib` again.\n"
    );
    if let Some(o) = doc["load_order"].as_object() {
        for (stage, units) in o {
            let list: Vec<String> = units.as_array().into_iter().flatten().filter_map(|u| u.as_str()).map(|u| u.trim_start_matches("scriptsbin/x2ui/").trim_end_matches("/toc.g").to_string()).collect();
            let _ = writeln!(w, "* **{stage}**: {}", list.join(", "));
        }
    }
    if let Some(runs) = doc["stats"]["runs"].as_array() {
        let _ = writeln!(w, "\nRecorder runs:\n\n| stage | scripts | scripts with errors | handler calls | handler errors |\n|---|---|---|---|---|");
        for r in runs {
            let _ = writeln!(w, "| {} | {} | {} | {} | {} |", r["stage"].as_str().unwrap_or(""), r["scripts"], r["load_errors"], r["handler_calls"], r["handler_failures"]);
        }
    }

    // 2. Toolkit
    let _ = writeln!(w, "\n## 2. Widget toolkit\n");
    let _ = writeln!(w, "### Widget types\n\nCreated with `UIParent:CreateWidget(type, id, parent)` or `parent:CreateChildWidget(type, id, index, ?)`. Ranked by creations in the recorder run.\n");
    let _ = writeln!(w, "| type | created | L | methods called on it (calls) |\n|---|---|---|---|");
    let mut widgets: Vec<(&String, &Json)> = doc["toolkit"]["widgets"].as_object().into_iter().flatten().collect();
    widgets.sort_by(|a, b| num(&b.1["created"]).cmp(&num(&a.1["created"])).then(a.0.cmp(b.0)));
    for (t, v) in &widgets {
        let mut ms: Vec<(&String, u64)> = v["members"].as_object().into_iter().flatten().map(|(k, m)| (k, num(&m["runtime_calls"]))).filter(|(_, c)| *c > 0).collect();
        ms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let list: Vec<String> = ms.iter().map(|(k, c)| format!("{k} ({c})")).collect();
        let _ = writeln!(w, "| `{t}` | {} | {} | {} |", v["created"], if v["login"].as_bool() == Some(true) { "L" } else { "" }, esc(&list.join(", ")));
    }
    let _ = writeln!(w, "\n### Widget fields and their methods\n\nNative sub-objects read from widgets (e.g. `button.style`), with the methods called on them.\n");
    let mut field_methods: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for (_, v) in &widgets {
        for (k, m) in v["field_members"].as_object().into_iter().flatten() {
            if let Some((field, meth)) = k.rsplit_once('.') {
                *field_methods.entry(field.to_string()).or_default().entry(meth.to_string()).or_default() += num(&m["runtime_calls"]).max(1);
            }
        }
    }
    for (field, ms) in &field_methods {
        let list: Vec<String> = top(ms, 60).into_iter().map(|(k, c)| format!("{k} ({c})")).collect();
        let _ = writeln!(w, "* `.{field}`: {}", list.join(", "));
    }
    let _ = writeln!(w, "\n### Drawables and other created objects\n\nReturned by native `Create*` methods (`widget:CreateImageDrawable(texture, layer)` …).\n");
    let _ = writeln!(w, "| object | created | created by | L | methods (calls) |\n|---|---|---|---|---|");
    let mut objs: Vec<(&String, &Json)> = doc["toolkit"]["objects"].as_object().into_iter().flatten().collect();
    objs.sort_by(|a, b| num(&b.1["created"]).cmp(&num(&a.1["created"])).then(a.0.cmp(b.0)));
    for (t, v) in &objs {
        let mut ms: Vec<(&String, u64)> = v["members"].as_object().into_iter().flatten().map(|(k, m)| (k, num(&m["runtime_calls"]))).filter(|(_, c)| *c > 0).collect();
        ms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let list: Vec<String> = ms.iter().map(|(k, c)| format!("{k} ({c})")).collect();
        let by: Vec<String> = v["created_by"].as_object().into_iter().flatten().map(|(k, _)| k.clone()).collect();
        let _ = writeln!(w, "| `{t}` | {} | {} | {} | {} |", v["created"], esc(&by.join(", ")), if v["login"].as_bool() == Some(true) { "L" } else { "" }, esc(&list.join(", ")));
    }
    let _ = writeln!(w, "\n### Handlers (`widget:SetHandler(name, fn)`)\n\n| handler | registrations | on |\n|---|---|---|");
    let mut hs: Vec<(&String, u64, Vec<String>)> = doc["toolkit"]["handlers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(h, by)| {
            let o = by.as_object();
            let total = o.map(|o| o.values().map(num).sum()).unwrap_or(0);
            let mut on: Vec<(u64, String)> = o.into_iter().flatten().map(|(k, v)| (num(v), k.clone())).collect();
            on.sort_by(|a, b| b.0.cmp(&a.0));
            (h, total, on.into_iter().take(8).map(|(n, k)| format!("{k} ({n})")).collect())
        })
        .collect();
    hs.sort_by(|a, b| b.1.cmp(&a.1));
    for (h, n, on) in hs {
        let _ = writeln!(w, "| `{h}` | {n} | {} |", esc(&on.join(", ")));
    }
    let _ = writeln!(w, "\n### All native methods called with `:` (static)\n\nEvery `obj:Method()` name in the scripts that the client registers natively, ranked by call sites, with the registered parameter names. Widget, drawable and X2 methods mixed (X2 methods are also listed per table in section 3).\n");
    let _ = writeln!(w, "| method | sites | argc | L | params | returns |\n|---|---|---|---|---|---|");
    for (m, v) in doc["toolkit"]["methods_static"].as_object().into_iter().flatten().collect::<Vec<_>>().iter().take(100000) {
        let params: Vec<String> = v["params"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).map(|p| format!("({p})")).collect();
        let _ = writeln!(w, "| {m} | {} | {} | {} | {} | {} |", v["sites"], argc_str(&v["argc"]), if v["login"].as_bool() == Some(true) { "L" } else { "" }, esc(&params.join(" ")), esc(&rets_str(&v["returns"])));
    }

    // 3. Game API
    let section = |w: &mut String, title: &str, list: &Json| {
        let _ = writeln!(w, "\n## {title}\n");
        let mut gl: Vec<(&String, &Json)> = list.as_object().into_iter().flatten().collect();
        gl.sort_by(|a, b| num(&b.1["sites"]).cmp(&num(&a.1["sites"])).then(a.0.cmp(b.0)));
        let _ = writeln!(w, "| global | sites | used members | L |\n|---|---|---|---|");
        for (g, v) in &gl {
            let used = v["members"].as_object().map(|o| o.values().filter(|m| num(&m["sites"]) > 0).count()).unwrap_or(0);
            let _ = writeln!(w, "| `{g}` | {} | {} | {} |", v["sites"], used, if v["login"].as_bool() == Some(true) { "L" } else { "" });
        }
        for (g, v) in &gl {
            let Some(ms) = v["members"].as_object() else { continue };
            let _ = writeln!(w, "\n### {g}\n\n| member | sites | argc | L | params | returns | types seen | string args seen |\n|---|---|---|---|---|---|---|---|");
            let mut ml: Vec<(&String, &Json)> = ms.iter().collect();
            ml.sort_by(|a, b| num(&b.1["sites"]).cmp(&num(&a.1["sites"])).then(a.0.cmp(b.0)));
            for (m, mv) in ml {
                let rt = &mv["runtime"];
                let types: Vec<String> = rt["arg_types"].as_object().into_iter().flatten().take(3).map(|(k, _)| format!("({k})")).collect();
                let strs: Vec<String> = rt["string_args"]["1"].as_array().into_iter().flatten().take(6).filter_map(|s| s.as_str()).map(|s| format!("`{}`", s.chars().take(40).collect::<String>())).collect();
                let unused = mv["unused"].as_bool() == Some(true);
                let not_native = mv["not_native"].as_bool() == Some(true);
                let _ = writeln!(
                    w,
                    "| {}{m} | {} | {} | {} | {} | {} | {} | {} |",
                    if unused { "~" } else if not_native { "!" } else { "" },
                    if unused { "-".to_string() } else { mv["sites"].to_string() },
                    argc_str(&mv["argc"]),
                    if mv["login"].as_bool() == Some(true) { "L" } else { "" },
                    esc(mv["params"].as_str().unwrap_or("")),
                    esc(&rets_str(&mv["returns"])),
                    esc(&types.join(" ")),
                    esc(&strs.join(" "))
                );
            }
        }
    };
    let _ = writeln!(
        w,
        "\nMember tables list every native function of the table in the real client (reference capture) plus whatever \
         the scripts use: `~name` = native but never used by the scripts, `!name` = used by the scripts but not a native \
         function of the table (nil in the client: a script bug, a guarded `if X.f then`, or a script-added field). \
         **params** come from the matching registration in `x2game.dll` (section 7)."
    );
    section(w, "3. X2 game API", &doc["game"]);
    section(w, "4. CryEngine API", &doc["cryengine"]);
    section(w, "5a. Toolkit globals", &doc["toolkit"]["globals"]);
    let _ = writeln!(w, "\n## 5. Other native functions and values\n\n### Functions\n\n| function | sites | argc | L | returns | types seen | string args seen |\n|---|---|---|---|---|---|---|");
    let mut fl: Vec<(&String, &Json)> = doc["functions"].as_object().into_iter().flatten().collect();
    fl.sort_by(|a, b| num(&b.1["sites"]).cmp(&num(&a.1["sites"])).then(a.0.cmp(b.0)));
    for (f, v) in fl {
        let rt = &v["runtime"];
        let types: Vec<String> = rt["arg_types"].as_object().into_iter().flatten().take(3).map(|(k, _)| format!("({k})")).collect();
        let strs: Vec<String> = rt["string_args"]["1"].as_array().into_iter().flatten().take(6).filter_map(|s| s.as_str()).map(|s| format!("`{s}`")).collect();
        let _ = writeln!(w, "| `{f}` | {} | {} | {} | {} | {} | {} |", v["sites"], argc_str(&v["argc"]), if v["login"].as_bool() == Some(true) { "L" } else { "" }, esc(&rets_str(&v["returns"])), esc(&types.join(" ")), esc(&strs.join(" ")));
    }
    let _ = writeln!(
        w,
        "\n### Values (constants, enums, tables)\n\nNative globals the scripts read but never assign or call (`ALIGN_CENTER`, \
         `UIC_PLAYER_UNITFRAME`, `RACE_NUIAN`, ...) with the value in the real client where captured (`= value`; tables in \
         `data/ui-api.json`). Format: `name` sites [L] [= value].\n"
    );
    let mut vl: Vec<(&String, &Json)> = doc["values"].as_object().into_iter().flatten().collect();
    vl.sort_by(|a, b| num(&b.1["sites"]).cmp(&num(&a.1["sites"])).then(a.0.cmp(b.0)));
    let items: Vec<String> = vl
        .iter()
        .map(|(n, v)| {
            let val = match &v["value"] {
                Json::Number(x) => format!(" = {x}"),
                Json::String(s) => format!(" = {:?}", s),
                Json::Null => String::new(),
                _ => " = {table}".to_string(),
            };
            format!("`{n}` {}{}{}", v["sites"], if v["login"].as_bool() == Some(true) { " L" } else { "" }, esc(&val))
        })
        .collect();
    let _ = writeln!(w, "{}", items.join(", "));
    let mut ul: Vec<(&String, &Json)> = doc["unresolved"].as_object().into_iter().flatten().collect();
    if !ul.is_empty() {
        ul.sort_by(|a, b| num(&b.1["sites"]).cmp(&num(&a.1["sites"])).then(a.0.cmp(b.0)));
        let items: Vec<String> = ul.iter().map(|(n, v)| format!("`{n}` {}{}", v["sites"], if v["login"].as_bool() == Some(true) { " L" } else { "" })).collect();
        let _ = writeln!(
            w,
            "\n### Read but absent in the real client\n\nGlobals the scripts read, no script assigns, and the real client's UI state \
             does not have at world select: script bugs or guarded reads (`if foo ~= nil`), or values only created in game. \
             Leave them nil unless the in-game capture shows otherwise.\n\n{}",
            items.join(", ")
        );
    }

    // 6. Events
    let _ = writeln!(w, "\n## 6. UI events\n\nThe client's event table has {} events (id = index, used by `FireUIEvent`; e.g. 70 `ENTERED_LOGIN`, 477 `ENTERED_WORLD_SELECT`, 78 `ENTERED_WORLD`). Scripts subscribe with `widget:RegisterEvent(name)` (usually via the script helper `RegistUIEvent(widget, {{NAME = fn}})`) and receive them in the widget's `OnEvent` handler as `(this, event, ...)`. Listed: events the scripts reference, by references; **reg** = registrations in the recorder run.\n", doc["events"].as_array().map(|a| a.len()).unwrap_or(0));
    let _ = writeln!(w, "| id | event | refs | reg | L |\n|---|---|---|---|---|");
    let mut ev: Vec<&Json> = doc["events"].as_array().into_iter().flatten().filter(|e| num(&e["script_refs"]) > 0 || num(&e["registered"]) > 0).collect();
    ev.sort_by(|a, b| num(&b["script_refs"]).cmp(&num(&a["script_refs"])).then(num(&a["id"]).cmp(&num(&b["id"]))));
    for e in &ev {
        let _ = writeln!(w, "| {} | `{}` | {} | {} | {} |", e["id"], e["name"].as_str().unwrap_or(""), e["script_refs"], e["registered"], if e["login"].as_bool() == Some(true) { "L" } else { "" });
    }
    let unused: Vec<String> = doc["events"].as_array().into_iter().flatten().filter(|e| num(&e["script_refs"]) == 0 && num(&e["registered"]) == 0).map(|e| format!("{} `{}`", e["id"], e["name"].as_str().unwrap_or(""))).collect();
    let _ = writeln!(w, "\nNever referenced by the scripts ({}): {}", unused.len(), unused.join(", "));
    if let Some(u) = doc["events_registered_unknown"].as_array()
        && !u.is_empty()
    {
        let _ = writeln!(w, "\nRegistered by scripts but not in the client's table: {}", u.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "));
    }

    // 7. bind groups
    let _ = writeln!(w, "\n## 7. Native bind groups (x2game.dll)\n\nEach script-bind class registers its functions in one function of `x2game.dll`. The owner is the real-client class (reference capture) whose native method list contains most of the group: an X2 table, `UIParent`, a widget type (its own methods), `widget` (the ~90 methods every widget type has), a drawable (`ImageDrawable` ...) or `style` (text style objects). Groups with owner `?` belong to the CryEngine/System script state, not the UI. Full lists with parameters are in `data/ui-api.json` (`native_bind_groups`).\n");
    let _ = writeln!(w, "| register fn | functions | owner guess (hits) | examples |\n|---|---|---|---|");
    for g in doc["native_bind_groups"].as_array().into_iter().flatten() {
        let f: Vec<String> = g["functions"].as_array().into_iter().flatten().take(6).filter_map(|p| p[0].as_str().map(String::from)).collect();
        let _ = writeln!(w, "| {} | {} | {} ({}) | {} |", g["register_fn"].as_str().unwrap_or(""), g["functions"].as_array().map(|a| a.len()).unwrap_or(0), g["owner_guess"].as_str().unwrap_or("?"), g["owner_hits"], esc(&f.join(", ")));
    }

    // 8. errors
    let _ = writeln!(w, "\n## 8. Run errors\n\nWhere the scripts stopped in the recorder run (all stages), most common first. They mark the proxy's limits (a native value that must be a real number, string or table) or a load-order dependency; the full list is in `data/ui-api.json` (`load_errors`).\n");
    for (e, n) in top(errors, 40) {
        let _ = writeln!(w, "* {n}x `{}`", esc(e));
    }
    md
}

/// Run everything and write the report files.
pub fn record_all(json_out: &std::path::Path, md_out: &std::path::Path) -> Result<()> {
    let files = ui_files();
    let scan = static_scan(&files)?;
    let x2game: Json = serde_json::from_str(X2GAME_JSON)?;
    let native_names: BTreeSet<String> = x2game["bind_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["functions"].as_array().into_iter().flatten().filter_map(|p| p[0].as_str().map(String::from)))
        .collect();
    let reference = load_reference(&reference_dir());
    let mut recs = Vec::new();
    for s in Stage::ALL {
        let r = record_stage(s, &scan, &native_names, reference.as_ref()).with_context(|| format!("recording {}", s.name()))?;
        eprintln!(
            "aa-lua record: {:17} {:4} scripts, {:3} with errors, {:5} handler calls ({} failed)",
            r.stage,
            r.scripts.len(),
            r.load_errors.len(),
            r.handler_calls,
            r.handler_failures
        );
        recs.push(r);
    }
    let (mut doc, mut md) = build_report(&Inputs { scan: &scan, recs: &recs, x2game: &x2game, reference: reference.as_ref() })?;
    // Load-order check against the real client (reference capture at world select).
    if let (Some(r), Some(ws)) = (reference.as_ref(), recs.iter().find(|r| r.stage == "world_select"))
        && !r.lua_functions.is_empty()
    {
        let missing: Vec<&String> = r.lua_functions.difference(&ws.defined_functions).collect();
        let extra: Vec<&String> = ws.defined_functions.difference(&r.lua_functions).collect();
        let both = r.lua_functions.intersection(&ws.defined_functions).count();
        doc["load_order_check"] = json!({
            "stage": "world_select",
            "real_client_functions": r.lua_functions.len(),
            "ours": ws.defined_functions.len(),
            "both": both,
            "missing_in_ours": missing,
            "extra_in_ours": extra,
        });
        md.push_str(&format!(
            "\n## 9. Load-order check\n\nGlobal script functions after loading world select: real client {} (reference capture), \
             ours {}, in both {}. Missing in ours ({}): {}. Extra in ours ({}): {}.\n",
            r.lua_functions.len(),
            ws.defined_functions.len(),
            both,
            missing.len(),
            missing.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", "),
            extra.len(),
            extra.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    std::fs::write(json_out, serde_json::to_string_pretty(&doc)?)?;
    std::fs::write(md_out, md)?;
    Ok(())
}
