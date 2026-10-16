//! The X2 widget tree: widgets and drawables ("objects"), their anchors, and anchor layout.
//!
//! Everything is in UI units (1 unit = 1 pixel at UI scale 1.0, origin top-left, y down).
//! Object 0 is `UIParent`, sized to the screen.

pub type Id = u32;

/// `UIParent`.
pub const ROOT: Id = 0;

/// The nine anchor points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Point {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Point {
    pub fn parse(s: &str) -> Option<Point> {
        Some(match s.to_ascii_uppercase().as_str() {
            "TOPLEFT" => Point::TopLeft,
            "TOP" => Point::Top,
            "TOPRIGHT" => Point::TopRight,
            "LEFT" => Point::Left,
            "CENTER" => Point::Center,
            "RIGHT" => Point::Right,
            "BOTTOMLEFT" => Point::BottomLeft,
            "BOTTOM" => Point::Bottom,
            "BOTTOMRIGHT" => Point::BottomRight,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Point::TopLeft => "TOPLEFT",
            Point::Top => "TOP",
            Point::TopRight => "TOPRIGHT",
            Point::Left => "LEFT",
            Point::Center => "CENTER",
            Point::Right => "RIGHT",
            Point::BottomLeft => "BOTTOMLEFT",
            Point::Bottom => "BOTTOM",
            Point::BottomRight => "BOTTOMRIGHT",
        }
    }

    /// Horizontal fraction: 0 left, 0.5 centre, 1 right.
    pub fn fx(self) -> f32 {
        match self {
            Point::TopLeft | Point::Left | Point::BottomLeft => 0.0,
            Point::Top | Point::Center | Point::Bottom => 0.5,
            _ => 1.0,
        }
    }

    /// Vertical fraction: 0 top, 0.5 centre, 1 bottom.
    pub fn fy(self) -> f32 {
        match self {
            Point::TopLeft | Point::Top | Point::TopRight => 0.0,
            Point::Left | Point::Center | Point::Right => 0.5,
            _ => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub point: Point,
    pub target: Id,
    pub rel: Point,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
}

/// Widget types (`CreateWidget` / `CreateChildWidget` type strings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetType {
    Root,
    EmptyWidget,
    Window,
    Button,
    CheckButton,
    Label,
    TextBox,
    EditBox,
    EditBoxMultiline,
    ListBox,
    ListCtrl,
    Slider,
    StatusBar,
    GameTooltip,
    Other(String),
}

impl WidgetType {
    pub fn parse(s: &str) -> WidgetType {
        match s.to_ascii_lowercase().as_str() {
            "emptywidget" => WidgetType::EmptyWidget,
            "window" => WidgetType::Window,
            "button" | "basebutton" => WidgetType::Button,
            "checkbutton" => WidgetType::CheckButton,
            "label" => WidgetType::Label,
            "textbox" => WidgetType::TextBox,
            "editbox" | "x2editbox" => WidgetType::EditBox,
            "editboxmultiline" => WidgetType::EditBoxMultiline,
            "listbox" => WidgetType::ListBox,
            "listctrl" => WidgetType::ListCtrl,
            "slider" => WidgetType::Slider,
            "statusbar" => WidgetType::StatusBar,
            "gametooltip" | "unitframetooltip" => WidgetType::GameTooltip,
            other => WidgetType::Other(other.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            WidgetType::Root => "root",
            WidgetType::EmptyWidget => "emptywidget",
            WidgetType::Window => "window",
            WidgetType::Button => "button",
            WidgetType::CheckButton => "checkbutton",
            WidgetType::Label => "label",
            WidgetType::TextBox => "textbox",
            WidgetType::EditBox => "editbox",
            WidgetType::EditBoxMultiline => "editboxmultiline",
            WidgetType::ListBox => "listbox",
            WidgetType::ListCtrl => "listctrl",
            WidgetType::Slider => "slider",
            WidgetType::StatusBar => "statusbar",
            WidgetType::GameTooltip => "gametooltip",
            WidgetType::Other(s) => s,
        }
    }

    /// Types that carry a text block (`SetText`, `style`). Windows use `title` / `titleStyle`.
    pub fn has_text(&self) -> bool {
        matches!(
            self,
            WidgetType::Button
                | WidgetType::CheckButton
                | WidgetType::Label
                | WidgetType::TextBox
                | WidgetType::EditBox
                | WidgetType::EditBoxMultiline
                | WidgetType::ListBox
                | WidgetType::GameTooltip
        )
    }

    /// Types that take mouse input even without script handlers.
    pub fn mouse_native(&self) -> bool {
        matches!(
            self,
            WidgetType::Button
                | WidgetType::CheckButton
                | WidgetType::EditBox
                | WidgetType::EditBoxMultiline
                | WidgetType::Slider
                | WidgetType::ListBox
        )
    }
}

/// `SetUILayer` names, bottom to top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Background,
    Game,
    Hud,
    #[default]
    Normal,
    Dialog,
    Tooltip,
    System,
}

impl Layer {
    pub fn parse(s: &str) -> Layer {
        match s.to_ascii_lowercase().as_str() {
            "background" => Layer::Background,
            "game" => Layer::Game,
            "hud" => Layer::Hud,
            "dialog" => Layer::Dialog,
            "tooltip" => Layer::Tooltip,
            "system" => Layer::System,
            _ => Layer::Normal,
        }
    }
}

/// Drawable layers inside one widget, bottom to top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrawLayer {
    #[default]
    Background,
    Artwork,
    Overlay,
}

impl DrawLayer {
    pub fn parse(s: &str) -> DrawLayer {
        match s.to_ascii_lowercase().as_str() {
            "artwork" => DrawLayer::Artwork,
            "overlay" | "highlight" => DrawLayer::Overlay,
            _ => DrawLayer::Background,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawKind {
    #[default]
    Image,
    NinePart,
    ThreePart,
    Color,
    Text,
    Icon,
}

/// Text alignment (`ALIGN_*`). The numeric values are what we expose to Lua.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    TopLeft = 0,
    Top = 1,
    TopRight = 2,
    #[default]
    Left = 3,
    Center = 4,
    Right = 5,
    BottomLeft = 6,
    Bottom = 7,
    BottomRight = 8,
}

impl Align {
    pub fn from_i(v: i64) -> Align {
        match v {
            0 => Align::TopLeft,
            1 => Align::Top,
            2 => Align::TopRight,
            3 => Align::Left,
            4 => Align::Center,
            5 => Align::Right,
            6 => Align::BottomLeft,
            7 => Align::Bottom,
            8 => Align::BottomRight,
            _ => Align::Left,
        }
    }
    pub fn fx(self) -> f32 {
        match self {
            Align::TopLeft | Align::Left | Align::BottomLeft => 0.0,
            Align::Top | Align::Center | Align::Bottom => 0.5,
            _ => 1.0,
        }
    }
    pub fn fy(self) -> f32 {
        match self {
            Align::TopLeft | Align::Top | Align::TopRight => 0.0,
            Align::Left | Align::Center | Align::Right => 0.5,
            _ => 1.0,
        }
    }
}

/// A widget's or text drawable's text style (`widget.style`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font: String,
    pub size: f32,
    pub align: Align,
    pub color: [f32; 4],
    pub shadow: bool,
    pub outline: bool,
    pub ellipsis: bool,
    pub snap: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font: crate::text::DEFAULT_FONT.to_string(),
            size: 13.0,
            align: Align::Left,
            color: [1.0, 1.0, 1.0, 1.0],
            shadow: false,
            outline: false,
            ellipsis: false,
            snap: true,
        }
    }
}

/// Text carried by a widget (labels, buttons, textboxes, window titles ...).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBlock {
    pub text: String,
    pub style: TextStyle,
    /// `SetInset(left, top, right, bottom)`.
    pub inset: [f32; 4],
    pub line_space: f32,
    /// Textboxes wrap at the widget width.
    pub wrap: bool,
    /// Gametooltip `AddLine` content (one entry per line).
    pub lines: Vec<String>,
}

/// Button visual state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonState {
    #[default]
    Normal,
    Highlight,
    Pushed,
    Disabled,
}

#[derive(Clone, Debug, Default)]
pub struct ButtonData {
    /// Drawables per state: normal, highlight, pushed, disabled.
    pub bg: [Option<Id>; 4],
    /// Text colours per state.
    pub text_color: [Option<[f32; 4]>; 4],
    pub checked: bool,
    pub checked_bg: Option<Id>,
}

/// Native list control state.
#[derive(Clone, Debug, Default)]
pub struct ListCtrlData {
    pub columns: Vec<Id>,
    pub column_widths: Vec<f32>,
    pub column_height: f32,
    pub rows: Vec<Id>,
    /// Sub-item widgets, `[row][col]`.
    pub sub_items: Vec<Vec<Id>>,
    /// Rows holding data (`InsertData`): only these show their sub-items and can be picked.
    pub has_data: Vec<bool>,
    pub row_height: f32,
    pub selected: i32,
    pub overed: i32,
    pub overed_image: Option<Id>,
    pub selected_image: Option<Id>,
}

#[derive(Clone, Debug, Default)]
pub struct SliderData {
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub step: f32,
    pub page_step: f32,
    pub vertical: bool,
    pub thumb: Option<Id>,
}

#[derive(Clone, Debug)]
pub struct Widget {
    pub ty: WidgetType,
    /// Native class name (`button`, `emptywidget`, ...), selects the Lua method table.
    pub class: String,
    pub children: Vec<Id>,
    pub drawables: Vec<Id>,
    pub layer: Layer,
    pub enabled: bool,
    pub clickable: bool,
    pub text: Option<TextBlock>,
    /// Window title text (`SetTitle`, `titleStyle`).
    pub title: Option<TextBlock>,
    pub button: Option<ButtonData>,
    pub list: Option<ListCtrlData>,
    pub slider: Option<SliderData>,
    pub modal: bool,
    /// Raise counter for sibling ordering (higher draws later).
    pub raise: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Drawable {
    pub kind: DrawKind,
    /// Native class (`drawable:image`, `drawable:ninepart`, ...).
    pub class: &'static str,
    pub layer: DrawLayer,
    pub texture: Option<String>,
    /// Texture rectangle `x, y, w, h` in texels; negative `w`/`h` mirror.
    pub coords: Option<[f32; 4]>,
    /// Nine/three part borders in texels (left, top, right, bottom).
    pub inset: [f32; 4],
    pub color: [f32; 4],
    /// Text drawables.
    pub text: Option<TextBlock>,
    /// Shown only while its owning button is in this state (state backgrounds).
    pub state_of: Option<(Id, ButtonState)>,
    /// Extra drawables of one button state (`CreateHighlightDrawable`, `CreateStateDrawable`):
    /// shown exactly while the owner is in that state, independent of the backgrounds.
    pub state_only: Option<ButtonState>,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Widget(Widget),
    Drawable(Drawable),
}

#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    /// Parent widget (for drawables: the owning widget).
    pub parent: Option<Id>,
    pub anchors: Vec<Anchor>,
    pub width: f32,
    pub height: f32,
    /// `Show` / `SetVisible`.
    pub shown: bool,
    pub alpha: f32,
    pub kind: Kind,
    /// Deleted objects keep their slot.
    pub alive: bool,
}

impl Object {
    pub fn widget(&self) -> Option<&Widget> {
        match &self.kind {
            Kind::Widget(w) => Some(w),
            _ => None,
        }
    }
    pub fn widget_mut(&mut self) -> Option<&mut Widget> {
        match &mut self.kind {
            Kind::Widget(w) => Some(w),
            _ => None,
        }
    }
    pub fn drawable(&self) -> Option<&Drawable> {
        match &self.kind {
            Kind::Drawable(d) => Some(d),
            _ => None,
        }
    }
    pub fn drawable_mut(&mut self) -> Option<&mut Drawable> {
        match &mut self.kind {
            Kind::Drawable(d) => Some(d),
            _ => None,
        }
    }
    pub fn is_widget(&self) -> bool {
        matches!(self.kind, Kind::Widget(_))
    }
    /// Text block of a widget or text drawable.
    pub fn text(&self) -> Option<&TextBlock> {
        match &self.kind {
            Kind::Widget(w) => w.text.as_ref(),
            Kind::Drawable(d) => d.text.as_ref(),
        }
    }
    pub fn text_mut(&mut self) -> Option<&mut TextBlock> {
        match &mut self.kind {
            Kind::Widget(w) => w.text.as_mut(),
            Kind::Drawable(d) => d.text.as_mut(),
        }
    }
}

/// All UI objects plus the layout cache.
pub struct Tree {
    pub objects: Vec<Object>,
    /// Top-level widgets in creation / raise order.
    pub top: Vec<Id>,
    pub screen_w: f32,
    pub screen_h: f32,
    raise_counter: u32,
    /// Bumped on every change that can move or restyle something.
    pub generation: u64,
    rects: std::cell::RefCell<(u64, Vec<Option<Rect>>)>,
}

impl Tree {
    pub fn new(screen_w: f32, screen_h: f32) -> Tree {
        let root = Object {
            name: "UIParent".into(),
            parent: None,
            anchors: Vec::new(),
            width: screen_w,
            height: screen_h,
            shown: true,
            alpha: 1.0,
            kind: Kind::Widget(Widget::new(WidgetType::Root)),
            alive: true,
        };
        Tree {
            objects: vec![root],
            top: Vec::new(),
            screen_w,
            screen_h,
            raise_counter: 0,
            generation: 1,
            rects: std::cell::RefCell::new((0, Vec::new())),
        }
    }

    pub fn touch(&mut self) {
        self.generation += 1;
    }

    pub fn set_screen(&mut self, w: f32, h: f32) {
        if (w, h) != (self.screen_w, self.screen_h) {
            self.screen_w = w;
            self.screen_h = h;
            self.objects[0].width = w;
            self.objects[0].height = h;
            self.touch();
        }
    }

    pub fn get(&self, id: Id) -> &Object {
        &self.objects[id as usize]
    }

    pub fn get_mut(&mut self, id: Id) -> &mut Object {
        self.generation += 1;
        &mut self.objects[id as usize]
    }

    pub fn widget(&self, id: Id) -> Option<&Widget> {
        self.objects.get(id as usize).and_then(Object::widget)
    }

    pub fn widget_mut(&mut self, id: Id) -> Option<&mut Widget> {
        self.generation += 1;
        self.objects.get_mut(id as usize).and_then(Object::widget_mut)
    }

    pub fn drawable_mut(&mut self, id: Id) -> Option<&mut Drawable> {
        self.generation += 1;
        self.objects.get_mut(id as usize).and_then(Object::drawable_mut)
    }

    /// Create a widget under `parent` (`None` or `ROOT` = top level).
    pub fn create_widget(&mut self, class: &str, name: &str, parent: Option<Id>) -> Id {
        let id = self.objects.len() as Id;
        let parent = parent.filter(|&p| p != ROOT && self.get(p).is_widget());
        let mut w = Widget::new(WidgetType::parse(class));
        w.class = class.to_ascii_lowercase();
        if let Some(p) = parent {
            w.layer = self.widget(p).map(|pw| pw.layer).unwrap_or_default();
        }
        self.raise_counter += 1;
        w.raise = self.raise_counter;
        self.objects.push(Object {
            name: name.to_string(),
            parent,
            anchors: Vec::new(),
            width: 0.0,
            height: 0.0,
            // Child widgets start shown, top-level windows hidden until `Show(true)`.
            shown: parent.is_some(),
            alpha: 1.0,
            kind: Kind::Widget(w),
            alive: true,
        });
        match parent {
            Some(p) => self.widget_mut(p).unwrap().children.push(id),
            None => self.top.push(id),
        }
        self.touch();
        id
    }

    pub fn create_drawable(&mut self, owner: Id, kind: DrawKind, class: &'static str, layer: DrawLayer) -> Id {
        let id = self.objects.len() as Id;
        self.objects.push(Object {
            name: String::new(),
            parent: Some(owner),
            anchors: Vec::new(),
            width: 0.0,
            height: 0.0,
            shown: true,
            alpha: 1.0,
            kind: Kind::Drawable(Drawable {
                kind,
                class,
                layer,
                color: [1.0; 4],
                text: (kind == DrawKind::Text).then(TextBlock::default),
                ..Default::default()
            }),
            alive: true,
        });
        if let Some(w) = self.widget_mut(owner) {
            w.drawables.push(id);
        }
        self.touch();
        id
    }

    /// Bring a widget to the front of its siblings.
    pub fn raise(&mut self, id: Id) {
        self.raise_counter += 1;
        let c = self.raise_counter;
        if let Some(w) = self.widget_mut(id) {
            w.raise = c;
        }
        let parent = self.get(id).parent;
        let list = match parent {
            Some(p) => &mut self.widget_mut(p).unwrap().children,
            None => &mut self.top,
        };
        if let Some(pos) = list.iter().position(|&c| c == id) {
            list.remove(pos);
            list.push(id);
        }
    }

    /// `AddAnchor`: an anchor on the same point replaces the old one.
    pub fn add_anchor(&mut self, id: Id, a: Anchor) {
        let o = self.get_mut(id);
        if let Some(old) = o.anchors.iter_mut().find(|o| o.point == a.point) {
            *old = a;
        } else {
            o.anchors.push(a);
        }
    }

    /// Effective visibility: shown and every ancestor shown.
    pub fn visible(&self, mut id: Id) -> bool {
        loop {
            let o = self.get(id);
            if !o.alive || !o.shown {
                return false;
            }
            match o.parent {
                Some(p) => id = p,
                None => return true,
            }
        }
    }

    /// Top-level ancestor of a widget (itself if top level).
    pub fn top_of(&self, mut id: Id) -> Id {
        while let Some(p) = self.get(id).parent {
            id = p;
        }
        id
    }

    pub fn is_descendant(&self, mut id: Id, ancestor: Id) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.get(id).parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    /// Resolved rectangle of any object.
    pub fn rect(&self, id: Id) -> Rect {
        let mut cache = self.rects.borrow_mut();
        if cache.0 != self.generation {
            cache.0 = self.generation;
            cache.1.clear();
        }
        if cache.1.len() < self.objects.len() {
            cache.1.resize(self.objects.len(), None);
        }
        let mut visiting = Vec::new();
        self.resolve(id, &mut cache.1, &mut visiting)
    }

    fn resolve(&self, id: Id, cache: &mut Vec<Option<Rect>>, visiting: &mut Vec<Id>) -> Rect {
        if let Some(r) = cache[id as usize] {
            return r;
        }
        let o = self.get(id);
        if id == ROOT {
            let r = Rect { x: 0.0, y: 0.0, w: self.screen_w, h: self.screen_h };
            cache[0] = Some(r);
            return r;
        }
        if visiting.contains(&id) {
            // Anchor cycle: fall back to the extent at the origin.
            return Rect { x: 0.0, y: 0.0, w: o.width, h: o.height };
        }
        visiting.push(id);
        let owner = o.parent.unwrap_or(ROOT);
        let mut anchors: Vec<Anchor> = o.anchors.clone();
        let mut fill = false;
        if anchors.is_empty() {
            if o.is_widget() || o.width > 0.0 || o.height > 0.0 {
                anchors.push(Anchor { point: Point::TopLeft, target: owner, rel: Point::TopLeft, x: 0.0, y: 0.0 });
            } else {
                fill = true;
            }
        }
        let r = if fill {
            self.resolve(owner, cache, visiting)
        } else {
            let (mut l, mut r, mut cx) = (None, None, None);
            let (mut t, mut b, mut cy) = (None, None, None);
            for a in &anchors {
                let tr = self.resolve(a.target, cache, visiting);
                let px = tr.x + tr.w * a.rel.fx() + a.x;
                let py = tr.y + tr.h * a.rel.fy() + a.y;
                match a.point.fx() {
                    f if f == 0.0 => l = Some(px),
                    f if f == 1.0 => r = Some(px),
                    _ => cx = Some(px),
                }
                match a.point.fy() {
                    f if f == 0.0 => t = Some(py),
                    f if f == 1.0 => b = Some(py),
                    _ => cy = Some(py),
                }
            }
            let (x, w) = axis(l, r, cx, o.width);
            let (y, h) = axis(t, b, cy, o.height);
            Rect { x, y, w, h }
        };
        visiting.pop();
        cache[id as usize] = Some(r);
        r
    }

    /// Width/height as scripts see them (`GetWidth`): anchored size if stretched, else extent.
    pub fn size(&self, id: Id) -> (f32, f32) {
        let r = self.rect(id);
        (r.w, r.h)
    }

    /// Widgets in paint order (bottom first), with effective visibility, by layer.
    pub fn paint_order(&self) -> Vec<Id> {
        let mut tops: Vec<Id> = self.top.iter().copied().filter(|&t| self.get(t).alive).collect();
        // Stable: layer first, then the existing (creation/raise) order.
        tops.sort_by_key(|&t| self.widget(t).map(|w| w.layer).unwrap_or_default());
        let mut out = Vec::with_capacity(self.objects.len());
        for t in tops {
            self.push_order(t, &mut out);
        }
        out
    }

    fn push_order(&self, id: Id, out: &mut Vec<Id>) {
        let o = self.get(id);
        if !o.alive || !o.shown {
            return;
        }
        out.push(id);
        if let Some(w) = o.widget() {
            for &c in &w.children {
                self.push_order(c, out);
            }
        }
    }

    /// Find a live widget by name (first match).
    pub fn find(&self, name: &str) -> Option<Id> {
        if name == "UIParent" {
            return Some(ROOT);
        }
        self.objects.iter().position(|o| o.alive && o.is_widget() && o.name == name).map(|i| i as Id)
    }
}

fn axis(lo: Option<f32>, hi: Option<f32>, center: Option<f32>, extent: f32) -> (f32, f32) {
    match (lo, hi, center) {
        (Some(l), Some(h), _) => (l, h - l),
        (Some(l), None, _) => (l, extent),
        (None, Some(h), _) => (h - extent, extent),
        (None, None, Some(c)) => (c - extent / 2.0, extent),
        (None, None, None) => (0.0, extent),
    }
}

impl Widget {
    pub fn new(ty: WidgetType) -> Widget {
        let text = ty.has_text().then(|| TextBlock {
            wrap: matches!(ty, WidgetType::TextBox | WidgetType::GameTooltip | WidgetType::EditBoxMultiline),
            style: TextStyle {
                align: if matches!(ty, WidgetType::Button | WidgetType::CheckButton) {
                    Align::Center
                } else if matches!(ty, WidgetType::TextBox | WidgetType::GameTooltip) {
                    Align::TopLeft
                } else {
                    Align::Left
                },
                ..Default::default()
            },
            ..Default::default()
        });
        let title = matches!(ty, WidgetType::Window).then(TextBlock::default);
        let button = matches!(ty, WidgetType::Button | WidgetType::CheckButton).then(ButtonData::default);
        let list = matches!(ty, WidgetType::ListCtrl).then(|| ListCtrlData {
            row_height: 25.0,
            column_height: 25.0,
            selected: -1,
            overed: -1,
            ..Default::default()
        });
        let slider = matches!(ty, WidgetType::Slider).then(|| SliderData { step: 1.0, page_step: 1.0, vertical: true, ..Default::default() });
        Widget {
            clickable: !matches!(ty, WidgetType::Label | WidgetType::TextBox),
            class: ty.name().to_string(),
            ty,
            children: Vec::new(),
            drawables: Vec::new(),
            layer: Layer::Normal,
            enabled: true,
            text,
            title,
            button,
            list,
            slider,
            modal: false,
            raise: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_resolve() {
        let mut t = Tree::new(1600.0, 900.0);
        let a = t.create_widget("emptywidget", "a", None);
        t.get_mut(a).width = 200.0;
        t.get_mut(a).height = 100.0;
        t.add_anchor(a, Anchor { point: Point::Center, target: ROOT, rel: Point::Center, x: 0.0, y: -60.0 });
        assert_eq!(t.rect(a), Rect { x: 700.0, y: 340.0, w: 200.0, h: 100.0 });
        let b = t.create_widget("button", "b", Some(a));
        t.add_anchor(b, Anchor { point: Point::TopLeft, target: a, rel: Point::TopLeft, x: 10.0, y: 10.0 });
        t.add_anchor(b, Anchor { point: Point::BottomRight, target: a, rel: Point::BottomRight, x: -10.0, y: -10.0 });
        assert_eq!(t.rect(b), Rect { x: 710.0, y: 350.0, w: 180.0, h: 80.0 });
        // Same point replaces.
        t.add_anchor(b, Anchor { point: Point::TopLeft, target: a, rel: Point::TopLeft, x: 0.0, y: 0.0 });
        assert_eq!(t.rect(b).w, 190.0);
    }
}
