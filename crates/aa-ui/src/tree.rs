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
        Rect {
            x,
            y,
            w: (r - x).max(0.0),
            h: (b - y).max(0.0),
        }
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
        match self {
            WidgetType::Button
            | WidgetType::CheckButton
            | WidgetType::Label
            | WidgetType::TextBox
            | WidgetType::EditBox
            | WidgetType::EditBoxMultiline
            | WidgetType::ListBox
            | WidgetType::GameTooltip => true,
            // Message frames (`custom_saveload.lua`: `wnd.style:SetAlign(...)`), combat texts,
            // cooldown buttons (buffs), folders (`SetViewOfFolder` sets `fld.style`).
            WidgetType::Other(s) => matches!(
                s.as_str(),
                "message"
                    | "folder"
                    | "listctrlitem"
                    | "damagedisplay"
                    | "cooldownbutton"
                    | "cooldownactionbutton"
                    | "cooldownconstantbutton"
                    | "cooldowninventorybutton"
            ),
            _ => false,
        }
    }

    /// Types with button states (normal / highlight / pushed / disabled backgrounds).
    pub fn is_button_like(&self) -> bool {
        match self {
            WidgetType::Button | WidgetType::CheckButton => true,
            WidgetType::Other(s) => matches!(
                s.as_str(),
                "slot"
                    | "cooldownbutton"
                    | "cooldownactionbutton"
                    | "cooldownconstantbutton"
                    | "cooldowninventorybutton"
            ),
            _ => false,
        }
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

/// `SetUILayer` names, bottom to top (native table39ac6a80; hit test392a8b65).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Background,
    Game,
    #[default]
    Normal,
    Hud,
    QuestDirecting,
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
            "questdirecting" => Layer::QuestDirecting,
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

/// Native fitting-prefix state, separate from the complete text and ellipsis style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoClipChar {
    pub enabled: bool,
    /// Native cached UTF-32 prefix length; enabling alone does not recalculate it.
    pub count: usize,
    /// Last resolved local rectangle and inherited scale (native geometry callbacks).
    pub rect: [f32; 4],
    pub scale: f32,
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
    /// `SetAutoResize(true)`: the widget width follows the text width.
    pub auto_resize: bool,
    pub auto_clip: AutoClipChar,
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

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonData {
    /// Native +2d0, retained independently of the current pointer snapshot.
    pub state: ButtonState,
    /// Native fresh initialization registers LeftButton, not Middle/RightButton.
    pub click_buttons: [bool; 3],
    /// Native +590: suppress this button's guarded input entries while its own +e0 is set.
    /// Hit testing, hover, focus, wheel and ancestor fades are independent.
    pub ignore_input_on_fading: bool,
    /// Drawables per state: normal, highlight, pushed, disabled.
    pub bg: [Option<Id>; 4],
    /// Native text palette, normal/highlight/pushed/disabled; applied on state writes.
    /// The current text style remains independently writable between those writes.
    pub text_color: [[f32; 4]; 4],
    pub checked: bool,
    pub checked_bg: Option<Id>,
    pub disabled_checked_bg: Option<Id>,
    /// `cooldownconstantbutton:SetSkill` observes timers without binding a clickable slot.
    pub cooldown_skill: Option<crate::lua::slot::ConstantSkillCooldown>,
    /// Generic cooldown mask texture and normalized source rectangle, independent of its timer.
    pub cooldown_mask: Option<crate::lua::slot::CooldownMask>,
}

impl Default for ButtonData {
    fn default() -> Self {
        Self {
            state: ButtonState::Normal,
            click_buttons: [true, false, false],
            ignore_input_on_fading: false,
            bg: [None; 4],
            text_color: [[1.; 4]; 4],
            checked: false,
            checked_bg: None,
            disabled_checked_bg: None,
            cooldown_skill: None,
            cooldown_mask: None,
        }
    }
}

/// A list control row key as scripts pass it to `InsertData` / `DeleteData`.
#[derive(Clone, Debug, PartialEq)]
pub enum ListKey {
    Num(f64),
    Str(String),
}

/// Native list control state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListCtrlData {
    pub columns: Vec<Id>,
    pub column_widths: Vec<f32>,
    /// `LCCIT_*` item type per column: 0 string, 1 button, 2 window.
    pub column_types: Vec<i64>,
    pub column_height: f32,
    pub item_line_gap: f32,
    pub rows: Vec<Id>,
    /// Sub-item widgets, `[row][col]`.
    pub sub_items: Vec<Vec<Id>>,
    /// Rows holding data (`InsertData`): only these show their sub-items and can be picked.
    pub has_data: Vec<bool>,
    /// The `InsertData` key that owns each row (`None` = free row). Scripts pass ids (a portal id,
    /// a mail id) rather than row numbers; a new key takes the lowest free row.
    pub keys: Vec<Option<ListKey>>,
    pub row_height: f32,
    /// Compatibility for the toolkit's nonnative `SetItemHeight` extension.
    pub explicit_row_height: Option<f32>,
    /// Last resolved local extent; bits also make nonfinite extents stable between ticks.
    pub layout_extent: Option<[u32; 2]>,
    pub selected: i32,
    pub overed: i32,
    pub overed_image: Option<Id>,
    pub selected_image: Option<Id>,
}

/// Native drawable-layer state, separate from a widget's whole-subtree alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicDrawableState {
    pub enabled: [bool; 4],
    /// Leave/over values; layer order is background, artwork, overlay, overoverlay.
    pub pairs: [[f32; 2]; 4],
    pub current: [f32; 4],
}

impl Default for DynamicDrawableState {
    fn default() -> Self {
        // Native layer constructor3927ec30: current1, leave0.2, over1.
        Self { enabled: [false; 4], pairs: [[0.2, 1.]; 4], current: [1.; 4] }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SliderData {
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub step: f32,
    /// `SetPageStep`: the visible part of the range (scroll windows pass their height).
    pub page_step: f32,
    pub vertical: bool,
    pub thumb: Option<Id>,
    /// `SetFixedThumb(true)`: the thumb keeps its own size.
    pub fixed_thumb: bool,
    /// `SetMinThumbLength`: with it (and no fixed thumb) the thumb's length along the track is
    /// `track * page / (range + page)`, at least this (the whole track while nothing scrolls).
    pub min_thumb: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelData {
    pub number_only: bool,
    /// Native GetText bytes when they differ from the decoded text (otherwise use TextBlock).
    pub raw: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub ty: WidgetType,
    /// Native class name (`button`, `emptywidget`, ...), selects the Lua method table.
    pub class: String,
    pub children: Vec<Id>,
    pub drawables: Vec<Id>,
    /// Native owner-layer draw admission (+108/+128/+148/+168), independent of
    /// each drawable's shown flag and the dynamic layer factors. The fourth
    /// flag is retained although fourth-layer drawable creation is unsupported.
    pub drawable_layers: [bool; 4],
    pub layer: Layer,
    pub dynamic_drawable: DynamicDrawableState,
    /// Native +dc/+e0: manager fade factor and pending fade, separate from base alpha/shown.
    pub fade_alpha: f32,
    pub fading: bool,
    pub enabled: bool,
    pub clickable: bool,
    pub text: Option<TextBlock>,
    pub label: Option<LabelData>,
    /// Window title text (`SetTitle`, `titleStyle`).
    pub title: Option<TextBlock>,
    pub button: Option<ButtonData>,
    pub list: Option<ListCtrlData>,
    pub listbox: Option<crate::lua::listbox::ListBoxData>,
    pub slider: Option<SliderData>,
    pub folder: Option<crate::lua::folder::FolderData>,
    pub tab: Option<crate::lua::tab::TabData>,
    pub edit: Option<crate::lua::edit::EditData>,
    pub diagram: Option<crate::lua::diagram::DiagramData>,
    pub picker: Option<crate::lua::picker::PickerData>,
    pub status: Option<crate::lua::statusbar::StatusBarData>,
    pub slot: Option<crate::lua::slot::SlotData>,
    pub message: Option<crate::lua::chat::MessageData>,
    pub chat: Option<crate::lua::chat::ChatData>,
    pub roadmap: Option<crate::lua::roadmap::RoadMapData>,
    /// `worldmap`: the registered maps, the shown one, the player's cursor (`crate::lua::worldmap`).
    pub worldmap: Option<crate::lua::worldmap::WorldMapData>,
    /// Published native radar rows and their drawable pools (Map widgets only).
    pub radar: Option<crate::lua::radar::View>,
    /// `EnableScroll(true)`: children are clipped to this widget's rectangle, and
    /// `ChangeChildAnchorByScrollValue` shifts the children anchored to it (`scroll`).
    pub clip: bool,
    /// Scroll offset in UI units (x, y): children anchored to this widget sit that much up / left.
    pub scroll: [f32; 2],
    /// A running cooldown sweep (a `slot` bound to a cooling skill or item, `SetCoolDown` of the
    /// cooldown buttons): `crate::lua::slot`.
    pub cooldown: Option<crate::lua::slot::Cooldown>,
    /// `gametooltip` lines (`AddLine`, ...): `crate::lua::tooltip`.
    pub tip: Option<crate::lua::tooltip::TooltipData>,
    /// A second text style without its own text (`slot.cooltime_style`, `listbox.childStyle`,
    /// `damagedisplay.extraStyle`, `editbox.prefixStyle`).
    pub aux_style: Option<TextBlock>,
    pub modal: bool,
    /// `SetCloseOnEscape(true)`: [`crate::UiRuntime::escape`] hides it (topmost first).
    pub close_on_escape: bool,
    /// `EnableHidingIsRemove(true)`: retire this widget after its hide completes.
    pub remove_on_hide: bool,
    /// Native user-resize borders and extents, independent of drag registrations.
    pub resizing: crate::lua::resize::Data,
    /// Raise counter for sibling ordering (higher draws later).
    pub raise: u32,
}

/// Native image/icon mask texture and its independent normalized rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawableMask {
    pub texture: String,
    pub uv: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawable {
    pub kind: DrawKind,
    /// Native class (`drawable:image`, `drawable:ninepart`, ...).
    pub class: &'static str,
    pub layer: DrawLayer,
    /// List-owned image template, rendered at each applicable row rather than once at its owner.
    pub row_template: bool,
    pub texture: Option<String>,
    /// Icon images: textures stacked over `texture` (`AddTexture`: icon, then frame / overlay).
    pub extra_textures: Vec<String>,
    pub mask: Option<DrawableMask>,
    /// Independent native byte: it can be set before a mask is loaded.
    pub mask_visible: bool,
    /// Texture rectangle `x, y, w, h` in texels; negative `w`/`h` mirror.
    pub coords: Option<[f32; 4]>,
    /// Nine/three part borders in texels (left, top, right, bottom).
    pub inset: [f32; 4],
    pub color: [f32; 4],
    /// Text drawables.
    pub text: Option<TextBlock>,
    /// Most recent button-background membership, retained as metadata.
    /// State application writes `Object::shown`; this is not a draw-time predicate.
    pub state_of: Option<(Id, ButtonState)>,
    /// Extra drawables of one button state (`CreateHighlightDrawable`, `CreateStateDrawable`):
    /// state application writes their visibility independently of the backgrounds.
    pub state_only: Option<ButtonState>,
    /// Status bar fill: layout uses this fraction, from the left (or top when vertical).
    pub fill: Option<(f32, bool)>,
    /// Effect drawables (`CreateEffectDrawable`): visible after `SetStartEffect(true)`;
    /// finite timelines retain their final sample until explicitly stopped.
    pub effect: Option<bool>,
    /// `SetRotation(r)`: radians, clockwise on screen, about the centre of the rect (image
    /// drawables; the roadmap's player cursor turns with the player's heading).
    pub rotation: f32,
}

#[derive(Clone, Debug, PartialEq)]
// Widgets (about 2 KB) are boxed, drawables (270 bytes, most of the objects) stay inline.
#[allow(clippy::large_enum_variant)]
pub enum Kind {
    Widget(Box<Widget>),
    Drawable(Drawable),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub name: String,
    /// Parent widget (for drawables: the owning widget).
    pub parent: Option<Id>,
    pub anchors: Vec<Anchor>,
    /// RemoveAllAnchors retains the last resolved geometry instead of moving to the owner.
    pub unanchored_origin: Option<[f32; 2]>,
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
            Kind::Widget(w) => Some(&**w),
            _ => None,
        }
    }
    pub fn widget_mut(&mut self) -> Option<&mut Widget> {
        match &mut self.kind {
            Kind::Widget(w) => Some(&mut **w),
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
    /// Widget ids by lower-case class, in creation order (destroyed ones included: check
    /// `alive`), for the per-frame passes over one kind (`tab`, `roadmap`, `chatwindow`, ...)
    /// without walking every object.
    by_class: std::collections::HashMap<String, Vec<Id>>,
    /// Native +44: dependants in first-target registration order, including replaced points
    /// until RemoveAllAnchors. This is independent of widget parenting and paint order.
    anchor_dependents: std::collections::HashMap<Id, Vec<Id>>,
    anchor_targets: std::collections::HashMap<Id, Vec<Id>>,
    anchor_pending: std::collections::HashSet<(Id, Id)>,
    /// Native +7c bypasses dependency readiness during an applied scale.
    anchor_scaling: std::collections::HashSet<Id>,
    /// During synchronous native propagation, getters see each target's last resolved bound.
    anchor_rects: std::collections::HashMap<Id, Rect>,
    anchor_depth: usize,
    /// See [`Tree::settle`].
    content: u64,
    dirty: bool,
    snaps: std::collections::HashMap<Id, Object>,
}

/// More objects written in one frame than this count as a change without comparing.
const MAX_SNAPSHOTS: usize = 2048;

impl Tree {
    pub fn new(screen_w: f32, screen_h: f32) -> Tree {
        let root = Object {
            name: "UIParent".into(),
            parent: None,
            anchors: Vec::new(),
            unanchored_origin: None,
            width: screen_w,
            height: screen_h,
            shown: true,
            alpha: 1.0,
            kind: Kind::Widget(Box::new(Widget::new(WidgetType::Root))),
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
            by_class: Default::default(),
            anchor_dependents: Default::default(),
            anchor_targets: Default::default(),
            anchor_pending: Default::default(),
            anchor_scaling: Default::default(),
            anchor_rects: Default::default(),
            anchor_depth: 0,
            content: 1,
            dirty: true,
            snaps: Default::default(),
        }
    }

    /// Live widgets of a class (lower case).
    pub fn of_class(&self, class: &str) -> Vec<Id> {
        self.by_class
            .get(class)
            .map(|v| v.iter().copied().filter(|&i| self.get(i).alive).collect())
            .unwrap_or_default()
    }

    /// A change that is not one object's own state (creation, the screen size, input state the
    /// drawing reads): the next [`Tree::settle`] counts it.
    #[track_caller]
    pub fn touch(&mut self) {
        self.generation += 1;
        if !self.dirty && crate::prof::enabled() {
            let at = std::panic::Location::caller();
            crate::prof::change(|| format!("touch {}:{}", at.file(), at.line()));
        }
        self.dirty = true;
    }

    /// Visual-only change (a drawable animation sample): repaint without invalidating anchors.
    #[track_caller]
    pub fn repaint(&mut self) {
        if !self.dirty && crate::prof::enabled() {
            let at = std::panic::Location::caller();
            crate::prof::change(|| format!("repaint {}:{}", at.file(), at.line()));
        }
        self.dirty = true;
    }

    /// Keep the state of `id` from before its first mutable access since the last settle, so
    /// [`Tree::settle`] can tell a real change from a write of the same values (the HUD scripts
    /// re-anchor, re-texture and re-show their widgets every frame).
    fn snapshot(&mut self, id: Id) {
        // Profiling keeps comparing after the first change, to list every changed object.
        if self.dirty && !crate::prof::enabled() {
            return;
        }
        if self.snaps.len() >= MAX_SNAPSHOTS {
            if !self.dirty {
                crate::prof::change(|| "more than MAX_SNAPSHOTS objects written".into());
            }
            self.dirty = true;
            self.snaps.clear();
            return;
        }
        if let Some(o) = self.objects.get(id as usize)
            && let std::collections::hash_map::Entry::Vacant(e) = self.snaps.entry(id)
        {
            e.insert(o.clone());
        }
    }

    /// The content version of the last [`Tree::settle`].
    pub fn content(&self) -> u64 {
        self.content
    }

    /// Whether something changed since the last [`Tree::settle`] (without settling).
    pub fn changed(&self) -> bool {
        self.dirty
            || self
                .snaps
                .iter()
                .any(|(id, old)| self.objects.get(*id as usize) != Some(old))
    }

    /// Fix the comparison point (a pass that read the layout): a change so
    /// far still counts at the next settle, later writes compare against the state from now.
    pub fn checkpoint(&mut self) {
        if crate::prof::enabled() {
            self.note_changes();
        }
        if !self.dirty && self.changed() {
            self.dirty = true;
        }
        self.snaps.clear();
    }

    /// Profiling: count every object whose state differs from its snapshot (`dirty objects`)
    /// and note each one by class, name and parent.
    fn note_changes(&self) {
        for (id, old) in &self.snaps {
            if self.objects.get(*id as usize) == Some(old) {
                continue;
            }
            crate::prof::count("dirty objects");
            let class = old
                .widget()
                .map_or(old.drawable().map_or("", |d| d.class), |w| w.class.as_str());
            let owner = old
                .parent
                .map(|p| self.objects[p as usize].name.clone())
                .unwrap_or_default();
            crate::prof::change(|| format!("object {class} {:?} (parent {owner})", old.name));
            if crate::prof::detail() {
                // `AA_UI_PROFILE=3`: what differs (the first differing span of the debug text).
                let (a, b) = (format!("{old:?}"), format!("{:?}", self.objects[*id as usize]));
                let mut i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
                while !a.is_char_boundary(i) {
                    i -= 1;
                }
                let lo = a[..i].rfind(',').map_or(0, |j| j + 1);
                let span = |t: &str| t.get(lo..(i + 40).min(t.len())).unwrap_or("").to_string();
                bevy::log::info!("aa-ui change {class} {:?}: {} -> {}", old.name, span(&a), span(&b));
            }
        }
    }

    /// The content version: advances when something changed since the last call (an object's
    /// state differs from before its first mutable access, or [`Tree::touch`]). The UI frame
    /// rebuilds the meshes when it moves; `generation` (every mutable access) only guards the
    /// layout cache.
    pub fn settle(&mut self) -> u64 {
        if crate::prof::enabled() {
            self.note_changes();
        }
        if !self.dirty && self.changed() {
            self.dirty = true;
        }
        if self.dirty {
            self.content += 1;
        }
        self.dirty = false;
        self.snaps.clear();
        self.content
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
        self.snapshot(id);
        &mut self.objects[id as usize]
    }

    pub fn widget(&self, id: Id) -> Option<&Widget> {
        self.objects.get(id as usize).and_then(Object::widget)
    }

    pub fn widget_mut(&mut self, id: Id) -> Option<&mut Widget> {
        self.generation += 1;
        self.snapshot(id);
        self.objects
            .get_mut(id as usize)
            .and_then(Object::widget_mut)
    }

    pub fn drawable_mut(&mut self, id: Id) -> Option<&mut Drawable> {
        self.generation += 1;
        self.snapshot(id);
        self.objects
            .get_mut(id as usize)
            .and_then(Object::drawable_mut)
    }

    /// Create a widget under `parent` (`None` or `ROOT` = top level).
    pub fn create_widget(&mut self, class: &str, name: &str, parent: Option<Id>) -> Id {
        let id = self.objects.len() as Id;
        let parent = parent.filter(|&p| p != ROOT && self.get(p).is_widget());
        let mut w = Widget::new(WidgetType::parse(class));
        w.class = class.to_ascii_lowercase();
        self.by_class.entry(w.class.clone()).or_default().push(id);
        if let Some(p) = parent {
            w.layer = self.widget(p).map(|pw| pw.layer).unwrap_or_default();
            // Native ordinary initialization does not clone its parent's enable flags.
            // Attachment3927e5d0 copies pairs from enabled ancestors, farther ones last.
            let mut ancestor = Some(p);
            while let Some(a) = ancestor {
                if let Some(pw) = self.widget(a) {
                    for layer in 0..4 {
                        if pw.dynamic_drawable.enabled[layer] {
                            w.dynamic_drawable.pairs[layer] = pw.dynamic_drawable.pairs[layer];
                            w.dynamic_drawable.current[layer] = pw.dynamic_drawable.pairs[layer][0];
                        }
                    }
                }
                ancestor = self.get(a).parent;
            }
        }
        self.raise_counter += 1;
        w.raise = self.raise_counter;
        self.objects.push(Object {
            name: name.to_string(),
            parent,
            anchors: Vec::new(),
            unanchored_origin: None,
            width: 0.0,
            height: 0.0,
            // Child widgets start shown, top-level windows hidden until `Show(true)`.
            shown: parent.is_some(),
            alpha: 1.0,
            kind: Kind::Widget(Box::new(w)),
            alive: true,
        });
        match parent {
            Some(p) => self.widget_mut(p).unwrap().children.push(id),
            None => self.top.push(id),
        }
        self.touch();
        id
    }

    pub fn create_drawable(
        &mut self,
        owner: Id,
        kind: DrawKind,
        class: &'static str,
        layer: DrawLayer,
    ) -> Id {
        let id = self.objects.len() as Id;
        self.objects.push(Object {
            name: String::new(),
            parent: Some(owner),
            anchors: Vec::new(),
            unanchored_origin: None,
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
        self.install_anchor(id, a);
    }

    /// Accepted setters notify even if the anchor already exists (native 39276dd0).
    pub(crate) fn install_anchor(&mut self, id: Id, a: Anchor) -> bool {
        // The same anchor does not dirty the object, but native still propagates it.
        if self.get(id).anchors.contains(&a) {
            self.prepare_anchor_change(id);
            return true;
        }
        // Like the client, an anchor that would make the object depend on itself is refused (the
        // old one stays): the tooltip scripts' `ResetSetctions` anchors every section below the
        // second last one, itself included, and the client keeps the chain `PushSection` built
        // (reference/ingame/tooltip_item).
        if self.anchor_cycle(id, a.target) {
            return false;
        }
        self.prepare_anchor_change(id);
        if !self
            .anchor_targets
            .get(&id)
            .is_some_and(|targets| targets.contains(&a.target))
        {
            self.invalidate_anchor_dependents(id);
            if !self.anchor_ready(a.target) {
                self.anchor_pending.insert((id, a.target));
            }
        }
        let dependents = self.anchor_dependents.entry(a.target).or_default();
        if !dependents.contains(&id) {
            dependents.push(id);
            self.anchor_targets.entry(id).or_default().push(a.target);
        }
        let o = self.get_mut(id);
        o.unanchored_origin = None;
        if let Some(old) = o.anchors.iter_mut().find(|o| o.point == a.point) {
            *old = a;
        } else {
            o.anchors.push(a);
        }
        true
    }

    pub(crate) fn anchor_dependent(&self, id: Id, index: usize) -> Option<Id> {
        self.anchor_dependents.get(&id)?.get(index).copied()
    }

    fn anchor_ready(&self, id: Id) -> bool {
        self.anchor_targets.get(&id).is_none_or(|targets| {
            targets
                .iter()
                .all(|&target| !self.anchor_pending.contains(&(id, target)))
        })
    }

    /// Native 39275ec0 marks the downstream dependency records unresolved.
    fn invalidate_anchor_dependents(&mut self, id: Id) {
        let mut stack = vec![id];
        while let Some(target) = stack.pop() {
            if let Some(dependents) = self.anchor_dependents.get(&target) {
                for &dependent in dependents {
                    if self.anchor_pending.insert((dependent, target)) {
                        stack.push(dependent);
                    }
                }
            }
        }
    }

    pub(crate) fn validate_anchor_target(&mut self, id: Id, target: Id) {
        self.anchor_pending.remove(&(id, target));
    }

    /// Preserve the bounds native has not reached yet, before a setter changes its source.
    pub(crate) fn prepare_anchor_change(&mut self, id: Id) {
        if self.anchor_depth == 0 {
            self.anchor_rects.clear();
        }
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let r = self.rect(id);
            self.anchor_rects.entry(id).or_insert(r);
            if let Some(dependents) = self.anchor_dependents.get(&id) {
                stack.extend(dependents);
            }
        }
    }

    pub(crate) fn begin_anchor_update(&mut self, id: Id) -> bool {
        if !self.anchor_scaling.contains(&id) && !self.anchor_ready(id) {
            return false;
        }
        self.anchor_depth += 1;
        self.anchor_rects.remove(&id);
        if let Some(r) = self.rects.get_mut().1.get_mut(id as usize) {
            *r = None;
        }
        let r = self.rect(id);
        self.anchor_rects.insert(id, r);
        true
    }

    pub(crate) fn anchor_scaling(&mut self, id: Id, active: bool) {
        if active {
            self.anchor_scaling.insert(id);
        } else {
            self.anchor_scaling.remove(&id);
        }
    }

    pub(crate) fn end_anchor_update(&mut self) {
        self.anchor_depth -= 1;
        if self.anchor_depth == 0 {
            // A leaf has no intermediate target state. Keep its ordinary cache: original
            // HUD scripts repeat thousands of unchanged drawable anchors every frame.
            if self.anchor_rects.len() > 1 {
                self.rects.get_mut().0 = 0;
            }
            self.anchor_rects.clear();
        }
    }

    /// Native 39275e70 rejects a dependency cycle, including retained target records.
    fn anchor_cycle(&self, id: Id, target: Id) -> bool {
        let mut stack = vec![id];
        let mut seen = Vec::new();
        while let Some(next) = stack.pop() {
            if next == target {
                return true;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(dependents) = self.anchor_dependents.get(&next) {
                stack.extend(dependents);
            }
        }
        false
    }

    /// Native 39276940 removes anchor/dependency records without resetting geometry.
    pub fn remove_anchors(&mut self, id: Id) {
        // A replaced point can retain a dependency on an old target without an anchor.
        for target in self.anchor_targets.remove(&id).unwrap_or_default() {
            self.anchor_pending.remove(&(id, target));
            if let Some(dependents) = self.anchor_dependents.get_mut(&target) {
                dependents.retain(|&dependent| dependent != id);
            }
        }
        if self.get(id).anchors.is_empty() {
            return;
        }
        let r = self.rect(id);
        let o = self.get_mut(id);
        o.anchors.clear();
        o.unanchored_origin = Some([r.x, r.y]);
        o.width = r.w;
        o.height = r.h;
    }

    /// Native 39276a80 detaches a retiring target before notifying its dependants.
    pub(crate) fn detach_anchor_target(&mut self, id: Id) -> Vec<Id> {
        self.remove_anchors(id);
        let dependents = self.anchor_dependents.remove(&id).unwrap_or_default();
        for &dependent in &dependents {
            self.anchor_pending.remove(&(dependent, id));
            if let Some(targets) = self.anchor_targets.get_mut(&dependent) {
                targets.retain(|&target| target != id);
            }
            let r = self.rect(dependent);
            let o = self.get_mut(dependent);
            o.anchors.retain(|anchor| anchor.target != id);
            if o.anchors.is_empty() {
                o.unanchored_origin = Some([r.x, r.y]);
                o.width = r.w;
                o.height = r.h;
            }
        }
        dependents
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
        if self.anchor_depth != 0 && let Some(&r) = self.anchor_rects.get(&id) {
            return r;
        }
        if let Some(r) = cache[id as usize] {
            return r;
        }
        let o = self.get(id);
        if id == ROOT {
            let r = Rect {
                x: 0.0,
                y: 0.0,
                w: self.screen_w,
                h: self.screen_h,
            };
            cache[0] = Some(r);
            return r;
        }
        if visiting.contains(&id) {
            // Anchor cycle: fall back to the extent at the origin.
            return Rect {
                x: 0.0,
                y: 0.0,
                w: o.width,
                h: o.height,
            };
        }
        visiting.push(id);
        let owner = o.parent.unwrap_or(ROOT);
        let mut anchors: Vec<Anchor> = o.anchors.clone();
        let mut fill = false;
        if anchors.is_empty() {
            if let Some([x, y]) = o.unanchored_origin {
                anchors.push(Anchor {
                    point: Point::TopLeft,
                    target: ROOT,
                    rel: Point::TopLeft,
                    x,
                    y,
                });
            } else {
                // Unanchored: at the owner's top left with its extent. Only text drawables without
                // an extent fill their owner; an image without anchors or extent is 0 x 0 like in
                // the client (`honorPointWarWnd.bg` in reference/ingame/spawn).
                let text = matches!(&o.kind, Kind::Drawable(d) if d.kind == DrawKind::Text);
                if o.is_widget() || o.width > 0.0 || o.height > 0.0 || !text {
                    // (An empty image drawable reports the screen origin, like the reference.)
                    let empty = !o.is_widget() && o.width == 0.0 && o.height == 0.0;
                    anchors.push(Anchor {
                        point: Point::TopLeft,
                        target: if empty { ROOT } else { owner },
                        rel: Point::TopLeft,
                        x: 0.0,
                        y: 0.0,
                    });
                } else {
                    fill = true;
                }
            }
        }
        let r = if fill {
            self.resolve(owner, cache, visiting)
        } else {
            let (mut l, mut r, mut cx) = (None, None, None);
            let (mut t, mut b, mut cy) = (None, None, None);
            // Two anchors that set the same edge: the one whose point comes first in
            // TOPLEFT, TOP, TOPRIGHT, LEFT, CENTER, RIGHT, BOTTOMLEFT, BOTTOM, BOTTOMRIGHT order
            // wins, whatever order they were added in (reference/ingame/bag: the tab line's
            // TOPLEFT y 410 beats its TOPRIGHT y 415; the slot grid's later TOPLEFT beats the
            // LEFT it was created with).
            let mut anchors = anchors;
            anchors.sort_by_key(|a| a.point as u8);
            for a in &anchors {
                let tr = self.resolve(a.target, cache, visiting);
                let mut px = tr.x + tr.w * a.rel.fx() + a.x;
                let mut py = tr.y + tr.h * a.rel.fy() + a.y;
                if a.target == owner {
                    // Children of a scrolled widget (`ChangeChildAnchorByScrollValue`).
                    if let Some(s) = self.widget(owner).map(|w| w.scroll) {
                        px -= s[0];
                        py -= s[1];
                    }
                }
                match a.point.fx() {
                    0.0 => l = l.or(Some(px)),
                    1.0 => r = r.or(Some(px)),
                    _ => cx = cx.or(Some(px)),
                }
                match a.point.fy() {
                    0.0 => t = t.or(Some(py)),
                    1.0 => b = b.or(Some(py)),
                    _ => cy = cy.or(Some(py)),
                }
            }
            let (x, w) = axis(l, r, cx, o.width);
            let (y, h) = axis(t, b, cy, o.height);
            Rect { x, y, w, h }
        };
        // A slider's thumb sits where the value is: along the track, centred across it; a
        // scroll bar's thumb is as long as the visible part of the range (reference/ingame/
        // options_keybinds: a 412 track, page 435 of 825 -> 217.2; nothing to scroll -> 437 of 437).
        let r = match self
            .widget(owner)
            .and_then(|p| p.slider.as_ref())
            .filter(|s| s.thumb == Some(id))
        {
            Some(s) => {
                let pr = self.resolve(owner, cache, visiting);
                let mut r = r;
                if !s.fixed_thumb && s.min_thumb > 0.0 {
                    let track = if s.vertical { pr.h } else { pr.w };
                    let range = (s.max - s.min).max(0.0);
                    let len = if s.page_step > 0.0 {
                        track * s.page_step / (range + s.page_step)
                    } else {
                        0.0
                    };
                    let len = len.max(s.min_thumb).min(track.max(s.min_thumb));
                    if s.vertical {
                        r.h = len;
                    } else {
                        r.w = len;
                    }
                }
                let f = if s.max > s.min {
                    ((s.value - s.min) / (s.max - s.min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if s.vertical {
                    Rect {
                        x: pr.x + (pr.w - r.w) / 2.0,
                        y: pr.y + f * (pr.h - r.h),
                        ..r
                    }
                } else {
                    Rect {
                        x: pr.x + f * (pr.w - r.w),
                        y: pr.y + (pr.h - r.h) / 2.0,
                        ..r
                    }
                }
            }
            None => r,
        };
        let mut r = r;
        if let Some((fraction, vertical)) = o.drawable().and_then(|d| d.fill) {
            if self.widget(owner).and_then(|w| w.status.as_ref())
                .is_some_and(|s| s.bar == Some(id) && s.bar_visible())
            {
                let owner = self.resolve(owner, cache, visiting);
                r.w = owner.w * if vertical { 1.0 } else { fraction };
                r.h = owner.h * if vertical { fraction } else { 1.0 };
            }
        }
        visiting.pop();
        cache[id as usize] = Some(r);
        r
    }

    /// The area an object is clipped to: the intersection of the rectangles of its ancestors
    /// that clip their children (`EnableScroll(true)`), `None` when nothing clips it.
    pub fn clip_rect(&self, id: Id) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut t = self.get(id).parent;
        while let Some(p) = t {
            if self.widget(p).is_some_and(|w| w.clip) {
                let r = self.rect(p);
                out = Some(out.map_or(r, |o| o.intersect(&r)));
            }
            t = self.get(p).parent;
        }
        out
    }

    /// Width/height as scripts see them (`GetWidth`): anchored size if stretched, else extent.
    pub fn size(&self, id: Id) -> (f32, f32) {
        let r = self.rect(id);
        (r.w, r.h)
    }

    /// Widgets in paint order (bottom first), with effective visibility, by layer.
    pub fn paint_order(&self) -> Vec<Id> {
        self.ordered_widgets(false)
    }

    /// Painting also admits hidden widgets whose native manager fade has not completed.
    /// Input and public visibility keep using the strict shown flags.
    pub fn fading_paint_order(&self) -> Vec<Id> {
        self.ordered_widgets(true)
    }

    fn ordered_widgets(&self, fading: bool) -> Vec<Id> {
        let mut tops: Vec<Id> = self
            .top
            .iter()
            .copied()
            .filter(|&t| self.get(t).alive)
            .collect();
        // Stable: layer first, then the existing (creation/raise) order.
        tops.sort_by_key(|&t| self.widget(t).map(|w| w.layer).unwrap_or_default());
        let mut out = Vec::with_capacity(self.objects.len());
        for t in tops {
            self.push_order(t, &mut out, fading);
        }
        out
    }

    fn push_order(&self, id: Id, out: &mut Vec<Id>, fading: bool) {
        let o = self.get(id);
        if !o.alive || !(o.shown || fading && o.widget().is_some_and(|w| w.fading)) {
            return;
        }
        out.push(id);
        if let Some(w) = o.widget() {
            for &c in &w.children {
                self.push_order(c, out, fading);
            }
        }
    }

    /// Find a live widget by name (first match).
    pub fn find(&self, name: &str) -> Option<Id> {
        if name == "UIParent" {
            return Some(ROOT);
        }
        self.objects
            .iter()
            .position(|o| o.alive && o.is_widget() && o.name == name)
            .map(|i| i as Id)
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
            wrap: matches!(
                ty,
                WidgetType::TextBox | WidgetType::GameTooltip | WidgetType::EditBoxMultiline
            ),
            style: TextStyle {
                // Text boxes centre too: world select's `warningLabel` (no `SetAlign`) is centred
                // in reference/world_select/real.png; every other script textbox sets its align.
                align: if matches!(
                    ty,
                    WidgetType::Button
                        | WidgetType::CheckButton
                        | WidgetType::Label
                        | WidgetType::TextBox
                ) {
                    Align::Center
                } else if matches!(ty, WidgetType::GameTooltip) {
                    Align::TopLeft
                } else {
                    Align::Left
                },
                ..Default::default()
            },
            ..Default::default()
        });
        // Window titles default to centred (`SetTitleInset` + `titleStyle:SetAlign` move them).
        let title = matches!(ty, WidgetType::Window).then(|| TextBlock {
            style: TextStyle {
                align: Align::Center,
                ..Default::default()
            },
            ..Default::default()
        });
        let button = ty.is_button_like().then(ButtonData::default);
        let list = matches!(ty, WidgetType::ListCtrl).then(|| ListCtrlData {
            column_height: 30.0,
            selected: -1,
            overed: -1,
            ..Default::default()
        });
        let slider = matches!(ty, WidgetType::Slider).then(|| SliderData {
            step: 1.0,
            page_step: 0.0,
            vertical: true,
            ..Default::default()
        });
        let listbox = (ty == WidgetType::ListBox).then(Default::default);
        let label = (ty == WidgetType::Label).then(LabelData::default);
        Widget {
            clickable: !matches!(ty, WidgetType::Label | WidgetType::TextBox),
            class: ty.name().to_string(),
            ty,
            children: Vec::new(),
            drawables: Vec::new(),
            drawable_layers: [true; 4],
            layer: Layer::Normal,
            dynamic_drawable: Default::default(),
            fade_alpha: 1.0,
            fading: false,
            enabled: true,
            text,
            label,
            title,
            button,
            list,
            listbox,
            slider,
            folder: None,
            tab: None,
            edit: None,
            diagram: None,
            picker: None,
            status: None,
            slot: None,
            message: None,
            chat: None,
            roadmap: None,
            worldmap: None,
            radar: None,
            clip: false,
            scroll: [0.0; 2],
            cooldown: None,
            tip: None,
            aux_style: None,
            modal: false,
            close_on_escape: false,
            remove_on_hide: false,
            resizing: Default::default(),
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
        t.add_anchor(
            a,
            Anchor {
                point: Point::Center,
                target: ROOT,
                rel: Point::Center,
                x: 0.0,
                y: -60.0,
            },
        );
        assert_eq!(
            t.rect(a),
            Rect {
                x: 700.0,
                y: 340.0,
                w: 200.0,
                h: 100.0
            }
        );
        let b = t.create_widget("button", "b", Some(a));
        t.add_anchor(
            b,
            Anchor {
                point: Point::TopLeft,
                target: a,
                rel: Point::TopLeft,
                x: 10.0,
                y: 10.0,
            },
        );
        t.add_anchor(
            b,
            Anchor {
                point: Point::BottomRight,
                target: a,
                rel: Point::BottomRight,
                x: -10.0,
                y: -10.0,
            },
        );
        assert_eq!(
            t.rect(b),
            Rect {
                x: 710.0,
                y: 350.0,
                w: 180.0,
                h: 80.0
            }
        );
        // Same point replaces.
        t.add_anchor(
            b,
            Anchor {
                point: Point::TopLeft,
                target: a,
                rel: Point::TopLeft,
                x: 0.0,
                y: 0.0,
            },
        );
        assert_eq!(t.rect(b).w, 190.0);
    }
}
