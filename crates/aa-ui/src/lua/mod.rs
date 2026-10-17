//! The Lua side of the toolkit: object tables, method dispatch, handlers and events.
//!
//! Every UI object (widget, drawable, text style) is a plain Lua table, so scripts can add
//! fields (`widget.style`, `window.titleBar = ...`) and override methods per instance
//! (`frame.InsertColumn = function ...`). Native methods live in one method table per object
//! kind, reached through the object's metatable `__index`. Ids are kept outside the tables (a
//! weak-keyed Lua table maps object table -> id), so `pairs(widget)` sees only script fields.
//!
//! Methods we know exist in the client but have not implemented are installed as logging stubs
//! (see `stubs`), so a script calling them keeps running; field lookups of unknown names still
//! return `nil`, as scripts test fields like `if self.bgs == nil`.

mod bounds;
pub mod chat;
pub mod chat_input;
pub mod classes;
pub mod constants;
pub mod diagram;
mod drawable;
pub mod edit;
pub mod folder;
mod globals;
pub mod grid;
pub mod picker;
pub mod roadmap;
pub mod slot;
pub mod statusbar;
mod stubs;
pub mod tab;
pub(crate) mod widget;
pub mod worldmap;

use std::collections::{BTreeMap, HashMap};

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::text::FontLib;
use crate::texture::TextureDb;
use crate::tree::{Id, Tree, WidgetType};

pub use globals::{install_game_stub, set_global_value};
pub use stubs::coverage;

/// Native state reachable from every Lua callback (stored as Lua app data).
pub struct Ui {
    pub tree: Tree,
    pub fonts: FontLib,
    pub textures: TextureDb,
    pub ui_scale: f32,
    /// Handler names set per widget (for hit testing and `OnUpdate`).
    pub handlers: HashMap<Id, Vec<String>>,
    /// The per-frame handlers (`OnUpdate`, `OnEffect`) by widget, kept with `handlers`, so the
    /// tick does not search every widget's list.
    pub frame_handlers: std::collections::BTreeSet<(Id, &'static str)>,
    /// Registered UI events -> widgets, in registration order.
    pub events: HashMap<String, Vec<Id>>,
    /// Handler calls queued by natives, run once the native returns.
    pub pending: Vec<(Id, String, Vec<Value>)>,
    pub fades: HashMap<Id, Fade>,
    pub time_ms: f64,
    pub frame_ms: f32,
    /// Calls to stubbed (unimplemented) methods: name -> count.
    pub stub_calls: BTreeMap<String, u32>,
    /// Widget under the mouse, keyboard focus.
    pub hover: Option<Id>,
    pub focus: Option<Id>,
    /// Caret blink state of the focused edit box and the time its phase started.
    pub caret_visible: bool,
    pub caret_t0: f64,
    pub cursor: (f32, f32),
    /// UI events raised from inside Lua (game API callbacks), delivered at the next tick like a
    /// server answer would be.
    pub queued_events: Vec<(String, Vec<Value>)>,
}

/// `Show(visible, ms)` fade.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub from: f32,
    pub to: f32,
    pub t: f32,
    pub duration: f32,
}

/// Lua-side registries (tables live in the Lua registry; handles are cheap to clone).
#[derive(Clone)]
pub struct Reg {
    /// id -> object table.
    pub objs: Table,
    /// object table -> id (weak keys).
    pub ids: Table,
    /// style table -> encoded (owner id * 2 + which) (weak keys).
    pub styles: Table,
    /// id -> { handlerName = function }.
    pub handlers: Table,
    /// Metatables by native class name (`button`, `drawable:image`, `uiparent`, ...).
    pub mt: HashMap<String, Table>,
    pub style_mt: Table,
}

pub fn ui(lua: &Lua) -> mlua::AppDataRefMut<'_, Ui> {
    lua.app_data_mut::<Ui>()
        .expect("aa-ui: Ui app data missing")
}

/// The registries. Shared (`Rc`): every native call looks its `self` up here, and cloning the
/// tables and the class map each time cost more than the natives themselves.
pub fn reg(lua: &Lua) -> std::rc::Rc<Reg> {
    lua.app_data_ref::<std::rc::Rc<Reg>>()
        .expect("aa-ui: Reg app data missing")
        .clone()
}

/// Widget classes the scripts create that `reference/api/widget_methods.json` does not list.
/// `worldmap` (the big map) and `listctrlitem` (list control sub items: text, `SetInset`) have
/// no capture name.
const EXTRA_CLASSES: &[&str] = &["paintcolorpicker", "worldmap", "listctrlitem"];

/// Install the toolkit into a Lua state: `UIParent`, widget/drawable classes, constants,
/// `Script.ReloadScript`, and stubs for the not yet implemented natives.
pub fn install(lua: &Lua, ui_state: Ui) -> mlua::Result<()> {
    lua.set_app_data(ui_state);
    let weak_keys = lua.create_table()?;
    weak_keys.raw_set("__mode", "k")?;
    let ids = lua.create_table()?;
    ids.set_metatable(Some(weak_keys.clone()))?;
    let styles = lua.create_table()?;
    styles.set_metatable(Some(weak_keys))?;

    let style_mt = lua.create_table()?;
    style_mt.raw_set("__index", widget::style_methods(lua)?)?;
    let mut mt = HashMap::new();
    for (class, _) in classes::CLASSES {
        let methods = if class.starts_with("drawable:") {
            drawable::methods(lua, class)?
        } else if *class == "textstyle" {
            continue;
        } else {
            widget::methods(lua, class)?
        };
        let m = lua.create_table()?;
        m.raw_set("__index", methods)?;
        mt.insert(class.to_string(), m);
    }
    // Classes the reference capture could not name: our natives plus stubs for every name.
    for class in EXTRA_CLASSES {
        let m = lua.create_table()?;
        m.raw_set("__index", widget::methods(lua, class)?)?;
        mt.insert(class.to_string(), m);
    }
    let r = Reg {
        objs: lua.create_table()?,
        ids,
        styles,
        handlers: lua.create_table()?,
        mt,
        style_mt,
    };
    lua.set_app_data(std::rc::Rc::new(r));
    // UIParent.
    let root = new_object_table(lua, 0)?;
    lua.globals().raw_set("UIParent", &root)?;
    // The client exposes the same root object as `UI` too.
    lua.globals().raw_set("UI", root)?;
    globals::install(lua)?;
    Ok(())
}

/// Metatable for widget classes the reference did not list (e.g. `unitframetooltip`).
fn fallback_class(ty: &WidgetType) -> &'static str {
    match ty {
        WidgetType::GameTooltip => "gametooltip",
        WidgetType::Button => "button",
        WidgetType::Window => "window",
        _ => "emptywidget",
    }
}

/// Create (or fetch) the Lua table of an object.
pub fn new_object_table(lua: &Lua, id: Id) -> mlua::Result<Table> {
    let r = reg(lua);
    if let Some(t) = r.objs.raw_get::<Option<Table>>(id)? {
        return Ok(t);
    }
    let t = lua.create_table()?;
    let (class, ty, has_text, has_title) = {
        let u = ui(lua);
        let o = u.tree.get(id);
        match (o.widget(), o.drawable()) {
            (Some(w), _) => (
                if id == 0 {
                    "uiparent".to_string()
                } else {
                    w.class.clone()
                },
                w.ty.clone(),
                w.text.is_some(),
                w.title.is_some(),
            ),
            (None, Some(d)) => (
                d.class.to_string(),
                WidgetType::Root,
                d.text.is_some(),
                false,
            ),
            _ => ("emptywidget".to_string(), WidgetType::Root, false, false),
        }
    };
    let meta =
        r.mt.get(&class)
            .or_else(|| r.mt.get(fallback_class(&ty)))
            .or_else(|| r.mt.get("emptywidget"))
            .cloned();
    t.set_metatable(meta)?;
    if has_text {
        let field = if ty == WidgetType::ListBox {
            "itemStyle"
        } else {
            "style"
        };
        t.raw_set(field, style_table(lua, id, 0)?)?;
    }
    if has_title {
        t.raw_set("titleStyle", style_table(lua, id, 1)?)?;
    }
    let aux = match class.as_str() {
        "listbox" => Some("childStyle"),
        "damagedisplay" => Some("extraStyle"),
        "editbox" | "x2editbox" => Some("prefixStyle"),
        _ => None,
    };
    if let Some(field) = aux {
        if let Some(w) = ui(lua).tree.widget_mut(id) {
            w.aux_style = Some(Default::default());
        }
        t.raw_set(field, style_table(lua, id, 2)?)?;
    }
    if ty == WidgetType::ListCtrl {
        t.raw_set("column", lua.create_table()?)?;
        t.raw_set("items", lua.create_table()?)?;
    }
    r.objs.raw_set(id, &t)?;
    r.ids.raw_set(&t, id)?;
    Ok(t)
}

/// A text style table of an object: `which` 0 = its text (`style`), 1 = a window title
/// (`titleStyle`), 2 = its auxiliary style ([`crate::tree::Widget::aux_style`]).
pub(crate) fn style_table(lua: &Lua, owner: Id, which: u32) -> mlua::Result<Table> {
    let r = reg(lua);
    let s = lua.create_table()?;
    s.set_metatable(Some(r.style_mt.clone()))?;
    r.styles.raw_set(&s, owner as i64 * 4 + which as i64)?;
    Ok(s)
}

/// The object table of an id (`nil` if never exposed).
pub fn object_table(lua: &Lua, id: Id) -> mlua::Result<Option<Table>> {
    reg(lua).objs.raw_get(id)
}

/// Resolve a Lua value naming an object: an object table, or a widget name (`"UIParent"`).
pub fn value_id(lua: &Lua, v: &Value) -> Option<Id> {
    match v {
        Value::Table(t) => reg(lua).ids.raw_get::<Option<Id>>(t).ok().flatten(),
        Value::String(s) => {
            let name = s.to_str().ok()?.to_string();
            ui(lua).tree.find(&name)
        }
        _ => None,
    }
}

/// Call a script handler (`OnClick`, `OnEvent` ...) on an object now. Errors are logged.
pub fn call_handler(lua: &Lua, id: Id, name: &str, args: Vec<Value>) {
    let r = reg(lua);
    let Ok(Some(hs)) = r.handlers.raw_get::<Option<Table>>(id) else {
        return;
    };
    let Ok(Some(f)) = hs.raw_get::<Option<Function>>(name) else {
        return;
    };
    let Ok(Some(this)) = r.objs.raw_get::<Option<Table>>(id) else {
        return;
    };
    let mut mv = MultiValue::new();
    mv.push_back(Value::Table(this));
    for a in args {
        mv.push_back(a);
    }
    if let Err(e) = f.call::<()>(mv) {
        bevy::log::warn!(
            "aa-ui: {name} handler of {}: {e}",
            ui(lua).tree.get(id).name
        );
    }
    flush(lua);
}

pub fn has_handler(lua: &Lua, id: Id, name: &str) -> bool {
    ui(lua)
        .handlers
        .get(&id)
        .is_some_and(|v| v.iter().any(|h| h == name))
}

/// Run handler calls queued by natives.
pub fn flush(lua: &Lua) {
    loop {
        let pending = std::mem::take(&mut ui(lua).pending);
        if pending.is_empty() {
            return;
        }
        for (id, name, args) in pending {
            call_handler(lua, id, &name, args);
        }
    }
}

/// Raise a UI event (`ENTERED_WORLD_SELECT`, ...): every widget that registered it gets
/// `OnEvent(self, name, args...)`.
pub fn fire_ui_event(lua: &Lua, name: &str, args: Vec<Value>) {
    let ids = ui(lua).events.get(name).cloned().unwrap_or_default();
    for id in ids {
        if id == 0 {
            // UIParent:SetEventHandler(name, fn): fn(...)
            let r = reg(lua);
            let f = r
                .handlers
                .raw_get::<Option<Table>>(0)
                .ok()
                .flatten()
                .and_then(|t| {
                    t.raw_get::<Option<Function>>(format!("__ev_{name}"))
                        .ok()
                        .flatten()
                });
            if let Some(f) = f {
                if let Err(e) = f.call::<()>(MultiValue::from_iter(args.iter().cloned())) {
                    bevy::log::warn!("aa-ui: event handler {name}: {e}");
                }
                flush(lua);
            }
            continue;
        }
        let mut a = vec![Value::String(lua.create_string(name).unwrap())];
        a.extend(args.iter().cloned());
        call_handler(lua, id, "OnEvent", a);
    }
}

/// Select a list control row (0-based, -1 none), optionally firing `OnSelChanged`.
pub fn widget_set_selected(lua: &Lua, list: Id, idx: i32, fire: bool, double: bool) {
    widget::set_selected(lua, list, idx, fire, double);
}

/// Anchor a list highlight drawable over a row (`None` hides it).
pub fn widget_place_on_row(u: &mut Ui, img: Id, row: Option<Id>) {
    widget::place_on_row(u, img, row);
}

/// Per-frame update: fades and `OnUpdate(self, dt_ms)` for visible widgets.
/// Queue a UI event for the next [`tick`] (safe to call from inside a native or a Lua callback).
pub fn queue_ui_event(lua: &Lua, name: &str, args: Vec<Value>) {
    ui(lua).queued_events.push((name.to_string(), args));
}

pub fn tick(lua: &Lua, dt_ms: f32) {
    use crate::prof::scope;
    {
        let _p = scope("tabs");
        tab::relayout_all(lua);
    }
    {
        let _p = scope("grids");
        grid::update(lua);
    }
    {
        let _p = scope("bounds");
        bounds::update(lua);
    }
    {
        let _p = scope("roadmap");
        roadmap::update(lua);
    }
    {
        let _p = scope("worldmap");
        worldmap::update(lua);
    }
    {
        let _p = scope("chat");
        chat::update(lua);
    }
    {
        let _p = scope("slots");
        slot::update(lua, dt_ms);
    }
    edit::tick(lua);
    let queued = std::mem::take(&mut ui(lua).queued_events);
    {
        let _p = scope("events");
        for (name, args) in queued {
            fire_ui_event(lua, &name, args);
        }
    }
    let p_fades = scope("fades");
    let (updates, ended) = {
        let mut u = ui(lua);
        u.time_ms += dt_ms as f64;
        u.frame_ms = dt_ms;
        let mut ended = Vec::new();
        let fades: Vec<(Id, Fade)> = u.fades.iter().map(|(k, v)| (*k, *v)).collect();
        for (id, mut f) in fades {
            f.t += dt_ms;
            let k = if f.duration <= 0.0 {
                1.0
            } else {
                (f.t / f.duration).min(1.0)
            };
            let a = f.from + (f.to - f.from) * k;
            let o = u.tree.get_mut(id);
            o.alpha = a;
            if k >= 1.0 {
                if f.to <= 0.0 {
                    o.shown = false;
                    o.alpha = 1.0;
                    ended.push((id, "OnEndFadeOut"));
                } else {
                    ended.push((id, "OnEndFadeIn"));
                }
                u.fades.remove(&id);
            } else {
                u.fades.insert(id, f);
            }
        }
        // `OnEffect(self, dt)` is the per-frame effect tick (the clock bar refreshes the zone name
        // and the sun there).
        let updates: Vec<(Id, &'static str)> = u
            .frame_handlers
            .iter()
            .copied()
            .filter(|&(id, _)| u.tree.visible(id))
            .collect();
        (updates, ended)
    };
    for (id, h) in ended {
        call_handler(lua, id, h, vec![]);
    }
    drop(p_fades);
    let _p = scope("on_update");
    for (id, h) in updates {
        let t0 = crate::prof::handler_start();
        call_handler(lua, id, h, vec![Value::Number(dt_ms as f64)]);
        crate::prof::handler_done(t0, || format!("{h} {}", ui(lua).tree.get(id).name));
    }
}

/// Argument helpers for natives (`args[0]` is `self` for methods).
pub(crate) struct Args(pub Vec<Value>);

impl Args {
    pub fn new(mv: MultiValue) -> Args {
        Args(mv.into_iter().collect())
    }
    pub fn v(&self, i: usize) -> Value {
        self.0.get(i).cloned().unwrap_or(Value::Nil)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn opt_f(&self, i: usize) -> Option<f32> {
        match self.0.get(i)? {
            Value::Integer(n) => Some(*n as f32),
            Value::Number(n) => Some(*n as f32),
            Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn f(&self, i: usize) -> f32 {
        self.opt_f(i).unwrap_or(0.0)
    }
    pub fn s(&self, i: usize) -> Option<String> {
        match self.0.get(i)? {
            Value::String(s) => Some(s.to_string_lossy()),
            Value::Integer(n) => Some(n.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    /// A boolean argument; missing = `default`. Numbers convert like the client's natives do
    /// (0 is false: the action bar options call `ShowActionBar(n, 0)` -> `Show(0)` hides).
    pub fn b(&self, i: usize, default: bool) -> bool {
        match self.0.get(i) {
            None | Some(Value::Nil) => default,
            Some(Value::Boolean(b)) => *b,
            Some(Value::Integer(n)) => *n != 0,
            Some(Value::Number(n)) => *n != 0.0,
            Some(_) => true,
        }
    }
    pub fn is_num(&self, i: usize) -> bool {
        matches!(self.0.get(i), Some(Value::Integer(_) | Value::Number(_)))
    }
}

/// Wrap a native method: resolves `self` to an id and passes the arguments.
pub(crate) fn method<F>(lua: &Lua, f: F) -> mlua::Result<Function>
where
    F: Fn(&Lua, Id, &Args) -> mlua::Result<MultiValue> + 'static,
{
    lua.create_function(move |lua, mv: MultiValue| {
        let args = Args::new(mv);
        let id = match args.0.first() {
            Some(Value::Table(t)) => reg(lua).ids.raw_get::<Option<Id>>(t)?,
            _ => None,
        };
        let Some(id) = id else {
            return Err(mlua::Error::runtime(
                "aa-ui: method called without a UI object as self",
            ));
        };
        let r = f(lua, id, &args);
        flush(lua);
        r
    })
}

pub(crate) fn ret<T: mlua::IntoLuaMulti>(lua: &Lua, v: T) -> mlua::Result<MultiValue> {
    v.into_lua_multi(lua)
}

pub(crate) fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}
