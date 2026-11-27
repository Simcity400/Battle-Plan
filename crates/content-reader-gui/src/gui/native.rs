use content_reader_script::localization_binding::BoundLocalization;

use std::{collections::BTreeMap, fmt, io};

mod elements;
pub mod fonts;
mod inherit;
pub mod keywords;
pub(crate) mod reader;
pub mod resources;
pub mod shader;
mod spill;
pub mod sprites;
mod windows;

pub use windows::{kept_container_window, window_button_sprite, window_button_tooltip, window_icon_sprite, window_texts};

use content_reader_script::native::Token;
pub use reader::{
    Axis as PointAxis, Case, PersistentKey, bool_of, fnv64 as tooltip_hash, persistent_keys,
};

use content_reader_layers::ResolvedLayers;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Refusal {
    pub path: String,
    pub key: String,
    pub reason: String,
    pub cause: Cause,
    node: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Cause {
    Unread,
    Unaligned,
    Unrecorded,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.path.is_empty(), self.key.is_empty()) {
            (true, true) => write!(f, "{}", self.reason),
            (true, false) => write!(f, "{}: {}", self.key, self.reason),
            (false, true) => write!(f, "{}: {}", self.path, self.reason),
            (false, false) => write!(f, "{}.{}: {}", self.path, self.key, self.reason),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Container,
    Icon,
    Button,
    Checkbox,
    GuiButton,
    MultiSpriteButton,
    TextBox,
    Grid,
    List,
    PositionType,
    ListBox,
    SmoothListBox,
    TextSprite,
    EditBox,
    Scrollbar,
    ExtendedScrollbar,
    DropDown,
    Browser,
    CurveGraph,
    Window,
    Other(&'static str),
}

impl Kind {
    pub fn keyword(&self) -> &'static str {
        match self {
            Self::Container => "containerWindowType",
            Self::Icon => "iconType",
            Self::Button => "buttonType",
            Self::Checkbox => "checkboxType",
            Self::GuiButton => "guiButtonType",
            Self::MultiSpriteButton => "multiSpriteButtonType",
            Self::TextBox => "instantTextBoxType",
            Self::Grid => "gridBoxType",
            Self::List => "OverlappingElementsBoxType",
            Self::PositionType => "positionType",
            Self::ListBox => "listBoxType",
            Self::SmoothListBox => "smoothListboxType",
            Self::TextSprite => "textBoxType",
            Self::EditBox => "editBoxType",
            Self::Scrollbar => "scrollbarType",
            Self::ExtendedScrollbar => "extendedScrollbarType",
            Self::DropDown => "dropDownBoxType",
            Self::Browser => "browserType",
            Self::CurveGraph => "curveGraphType",
            Self::Window => "windowType",
            Self::Other(keyword) => keyword,
        }
    }

    pub fn lookup(&self) -> &'static str {
        match self {
            Self::Container => "container lookup",
            Self::Icon => "icon lookup",
            Self::Button => "button lookup",
            Self::TextBox => "text box lookup",
            Self::Grid => "grid lookup",
            Self::List => "list lookup",
            Self::PositionType => "position instance lookup",
            _ => "no lookup this reader models",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
    Center,
    CenterLeft,
    CenterRight,
    CenterUp,
    CenterDown,
}

impl Orientation {
    const NAMES: [(&'static str, Self); 9] = [
        ("upper_left", Self::UpperLeft),
        ("upper_right", Self::UpperRight),
        ("lower_left", Self::LowerLeft),
        ("lower_right", Self::LowerRight),
        ("center", Self::Center),
        ("center_left", Self::CenterLeft),
        ("center_right", Self::CenterRight),
        ("center_up", Self::CenterUp),
        ("center_down", Self::CenterDown),
    ];

    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, own)| *own == self)
            .map_or("upper_left", |(name, _)| name)
    }

    pub fn from_value(value: i32) -> Option<Self> {
        Self::NAMES
            .get(usize::try_from(value).ok()?)
            .map(|(_, own)| *own)
    }

    pub fn gui_value(self) -> i32 {
        match self {
            Self::UpperLeft | Self::CenterLeft | Self::CenterRight => 0,
            Self::UpperRight => 1,
            Self::LowerLeft => 2,
            Self::LowerRight => 3,
            Self::Center => 4,
            Self::CenterUp => 5,
            Self::CenterDown => 6,
        }
    }

    fn from_keyword(token: &Token) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(name, _)| token.is_keyword(name))
            .map(|(_, own)| *own)
    }

    fn parse_gui(text: &[u8]) -> Self {
        Self::NAMES
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, own)| *own)
            .filter(|own| !matches!(own, Self::CenterLeft | Self::CenterRight))
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextFormat {
    #[default]
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListFormat {
    Center,
    Right,
    #[default]
    Left,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridFormat {
    #[default]
    Up,
    Down,
    Left,
    Right,
    UpperLeft,
    LowerLeft,
    UpperRight,
}

impl GridFormat {
    const VALUES: [Self; 7] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::UpperLeft,
        Self::LowerLeft,
        Self::UpperRight,
    ];

    pub fn from_value(value: i32) -> Self {
        usize::try_from(value)
            .ok()
            .and_then(|index| Self::VALUES.get(index))
            .copied()
            .unwrap_or(Self::Up)
    }

    fn parse(text: &[u8]) -> i32 {
        const NAMES: [&[u8]; 7] = [
            b"UP",
            b"DOWN",
            b"LEFT",
            b"RIGHT",
            b"UPPER_LEFT",
            b"LOWER_LEFT",
            b"UPPER_RIGHT",
        ];
        NAMES
            .iter()
            .position(|name| *name == text)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Animation {
    #[default]
    None,
    Accelerated,
    Decelerated,
    Smoothstep,
    Linear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coordinate {
    pub value: i32,
    pub percent: bool,
    pub from_position: bool,
    pub auto: bool,
}

impl Coordinate {
    pub const UNBOUNDED: Self = Self {
        value: 0,
        percent: false,
        from_position: false,
        auto: true,
    };

    pub fn pixels(value: i32) -> Self {
        Self {
            value,
            ..Self::default()
        }
    }
}

const STEP: [Coordinate; 2] = [Coordinate {
    value: 1,
    percent: false,
    from_position: false,
    auto: false,
}; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: Coordinate,
    pub y: Coordinate,
    pub min: [Coordinate; 2],
    pub max: [Coordinate; 2],
    pub step: [Coordinate; 2],
}

impl Default for Position {
    fn default() -> Self {
        Self {
            x: Coordinate::default(),
            y: Coordinate::default(),
            min: [Coordinate::UNBOUNDED; 2],
            max: [Coordinate::UNBOUNDED; 2],
            step: STEP,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub size: [Coordinate; 2],
    pub min: [Coordinate; 2],
    pub max: [Coordinate; 2],
    pub step: [Coordinate; 2],
    pub preserve_aspect_ratio: bool,
}

impl Default for Size {
    fn default() -> Self {
        Self {
            size: [Coordinate::default(); 2],
            min: [Coordinate::UNBOUNDED; 2],
            max: [Coordinate::UNBOUNDED; 2],
            step: STEP,
            preserve_aspect_ratio: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub left: Coordinate,
    pub top: Coordinate,
    pub right: Coordinate,
    pub bottom: Coordinate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipAnchor {
    pub set: bool,
    pub x: i32,
    pub y: i32,
    pub relative: bool,
    pub below: bool,
}

impl Default for TooltipAnchor {
    fn default() -> Self {
        Self {
            set: false,
            x: 0,
            y: 0,
            relative: true,
            below: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Common {
    pub tooltip: u64,
    pub tooltip_key: Vec<u8>,
    pub delayed_tooltip: u64,
    pub delayed_tooltip_key: Vec<u8>,
    pub disabled_tooltip: u64,
    pub disabled_tooltip_key: Vec<u8>,
    pub disabled_tooltip_delayed: u64,
    pub disabled_tooltip_delayed_key: Vec<u8>,
    pub hint_tag: Vec<u8>,
    pub tooltip_anchor: TooltipAnchor,
    pub bound_tooltip: BoundLocalization,
    pub context_aware_tooltip: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub color: [f32; 4],
    pub font: Vec<u8>,
    pub text: Vec<u8>,
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub format: i32,
}

impl Default for TextBlock {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            font: Vec::new(),
            text: Vec::new(),
            position: [0; 2],
            size: [0; 2],
            format: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub position: Position,
    pub show_position: Position,
    pub hide_position: Position,
    pub size: Size,
    pub orientation: Orientation,
    pub origo: Orientation,
    pub clipping: Option<bool>,
    pub margin: Margin,
    pub background_margin: Margin,
    pub backgrounds: Vec<Element>,
    pub scale: f32,
    pub show_animation: Animation,
    pub hide_animation: Animation,
    pub fade: Animation,
    pub fade_time: i32,
    pub fade_delay: i32,
    pub show_animation_time: i32,
    pub hide_animation_time: i32,
    pub moveable: bool,
    pub autohide_scrollbars: bool,
    pub vertical_scrollbar: Vec<u8>,
    pub horizontal_scrollbar: Vec<u8>,
    pub click_to_front: bool,
    pub smooth_scrolling: bool,
    pub scroll_wheel_factor: f32,
    pub vertical_scroll_step: f32,
    pub horizontal_scroll_step: f32,
    pub drag_scroll: u8,
    pub closed_position: Position,
    pub open_position: Position,
    pub peek_position: Position,
    pub peek_check_top_left: Position,
    pub peek_check_size: Size,
    pub peek: bool,
    pub show_sound: Vec<u8>,
    pub hide_sound: Vec<u8>,
}

impl Default for Container {
    fn default() -> Self {
        Self {
            position: Position::default(),
            show_position: Position::default(),
            hide_position: Position::default(),
            size: Size::default(),
            orientation: Orientation::UpperLeft,
            origo: Orientation::UpperLeft,
            clipping: None,
            margin: Margin::default(),
            background_margin: Margin::default(),
            backgrounds: Vec::new(),
            scale: 1.0,
            show_animation: Animation::None,
            hide_animation: Animation::None,
            fade: Animation::None,
            fade_time: 1000,
            fade_delay: 0,
            show_animation_time: 10,
            hide_animation_time: 10,
            moveable: false,
            autohide_scrollbars: true,
            vertical_scrollbar: Vec::new(),
            horizontal_scrollbar: Vec::new(),
            click_to_front: false,
            smooth_scrolling: false,
            scroll_wheel_factor: 15.0,
            vertical_scroll_step: 1.0,
            horizontal_scroll_step: 1.0,
            drag_scroll: 0,
            closed_position: Position::default(),
            open_position: Position::default(),
            peek_position: Position::default(),
            peek_check_top_left: Position::default(),
            peek_check_size: Size::default(),
            peek: false,
            show_sound: Vec::new(),
            hide_sound: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub sprite: Vec<u8>,
    pub position: [f32; 2],
    pub scale: f32,
    pub orientation: Orientation,
    pub frame: i32,
    pub rotation: f32,
    pub always_transparent: bool,
    pub center_position: bool,
    pub center_on_anchor: bool,
}

impl Default for Icon {
    fn default() -> Self {
        Self {
            sprite: Vec::new(),
            position: [0.0; 2],
            scale: 1.0,
            orientation: Orientation::UpperLeft,
            frame: 1,
            rotation: 0.0,
            always_transparent: false,
            center_position: false,
            center_on_anchor: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub sprite: Vec<u8>,
    pub position: [i32; 2],
    pub scale: f32,
    pub rotation: f32,
    pub frame: i32,
    pub orientation_text: Vec<u8>,
    pub orientation: Option<Orientation>,
    pub center_position: bool,
    pub always_transparent: bool,
    pub enabled: bool,
    pub font: Vec<u8>,
    pub text: Vec<u8>,
    pub format: TextFormat,
    pub vertical_alignment: VerticalAlignment,
    pub multiline: bool,
    pub size: [i32; 2],
    pub border_size: [i32; 2],
    pub parent: Vec<u8>,
    pub up_sprite: Vec<u8>,
    pub down_sprite: Vec<u8>,
    pub over_sprite: Vec<u8>,
    pub dissable_sprite: Vec<u8>,
    pub shortcuts: Vec<Vec<u8>>,
    pub lua_click: Vec<u8>,
    pub callback_type: Vec<u8>,
    pub callback_argument: Vec<u8>,
    pub up_sound: Vec<u8>,
    pub down_sound: Vec<u8>,
    pub over_sound: Vec<u8>,
    pub click_sound: Vec<u8>,
    pub no_click_sound: bool,
}

impl Default for Button {
    fn default() -> Self {
        Self {
            sprite: Vec::new(),
            position: [0; 2],
            scale: 1.0,
            rotation: 0.0,
            frame: 0,
            orientation_text: b"UPPER_LEFT".to_vec(),
            orientation: None,
            center_position: false,
            always_transparent: false,
            enabled: true,
            font: Vec::new(),
            text: Vec::new(),
            format: TextFormat::Center,
            vertical_alignment: VerticalAlignment::Center,
            multiline: false,
            size: [0; 2],
            border_size: [0; 2],
            parent: Vec::new(),
            up_sprite: Vec::new(),
            down_sprite: Vec::new(),
            over_sprite: Vec::new(),
            dissable_sprite: Vec::new(),
            shortcuts: Vec::new(),
            lua_click: Vec::new(),
            callback_type: Vec::new(),
            callback_argument: Vec::new(),
            up_sound: Vec::new(),
            down_sound: Vec::new(),
            over_sound: Vec::new(),
            click_sound: Vec::new(),
            no_click_sound: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub font: Vec<u8>,
    pub position: [i32; 2],
    pub format: TextFormat,
    pub max_width: u32,
    pub max_height: u32,
    pub text: Vec<u8>,
    pub orientation: Orientation,
    pub vertical_alignment: VerticalAlignment,
    pub always_transparent: bool,
    pub border_size: [i32; 2],
    pub scrollbar_type: Vec<u8>,
    pub fixed_size: bool,
    pub truncate: bool,
    pub multiline: bool,
    pub text_color_code: u8,
    pub ignore_special_chars: bool,
    pub bound_text: BoundLocalization,
    pub context_aware: bool,
}

impl Default for TextBox {
    fn default() -> Self {
        Self {
            font: Vec::new(),
            position: [0; 2],
            format: TextFormat::Left,
            max_width: 0,
            max_height: 0,
            text: Vec::new(),
            orientation: Orientation::UpperLeft,
            vertical_alignment: VerticalAlignment::Top,
            always_transparent: false,
            border_size: [0; 2],
            scrollbar_type: Vec::new(),
            fixed_size: false,
            truncate: true,
            multiline: true,
            text_color_code: 0,
            ignore_special_chars: false,
            bound_text: BoundLocalization::default(),
            context_aware: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub position: Position,
    pub size: Size,
    pub slot_size: Size,
    pub format: i32,
    pub orientation: i32,
    pub max_slots: [i32; 2],
    pub add_horizontal: bool,
    pub padding: Margin,
    pub background: Option<Box<Element>>,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            slot_size: Size::default(),
            format: 0,
            orientation: 0,
            max_slots: [-1; 2],
            add_horizontal: true,
            padding: Margin::default(),
            background: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub parent: Vec<u8>,
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub format: ListFormat,
    pub spacing: i32,
    pub first_on_top: bool,
    pub orientation: Orientation,
}

impl Default for List {
    fn default() -> Self {
        Self {
            parent: Vec::new(),
            position: [0; 2],
            size: [0; 2],
            format: ListFormat::Left,
            spacing: 1,
            first_on_top: false,
            orientation: Orientation::UpperLeft,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionType {
    pub position: [i32; 2],
    pub scale: f32,
}

impl Default for PositionType {
    fn default() -> Self {
        Self {
            position: [0; 2],
            scale: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListBox {
    pub offset: [i32; 2],
    pub size: [i32; 2],
    pub position: [i32; 2],
    pub border_size: [i32; 2],
    pub parent: Vec<u8>,
    pub scrollbar_type: Vec<u8>,
    pub priority: Option<f32>,
    pub horizontal: i32,
    pub background: Vec<u8>,
    pub drop_down: Vec<u8>,
    pub heading: Vec<u8>,
    pub spacing: i32,
    pub orientation: Vec<u8>,
    pub member_name: Vec<u8>,
    pub always_transparent: bool,
    pub clipping: bool,
    pub scrollbar_side: i32,
}

impl Default for ListBox {
    fn default() -> Self {
        Self {
            offset: [0; 2],
            size: [0; 2],
            position: [0; 2],
            border_size: [0; 2],
            parent: Vec::new(),
            scrollbar_type: Vec::new(),
            priority: None,
            horizontal: 0,
            background: Vec::new(),
            drop_down: Vec::new(),
            heading: Vec::new(),
            spacing: 1,
            orientation: Vec::new(),
            member_name: Vec::new(),
            always_transparent: false,
            clipping: true,
            scrollbar_side: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditBox {
    pub font: Vec<u8>,
    pub texture: Vec<u8>,
    pub text: Vec<u8>,
    pub slots: [i32; 12],
}

impl EditBox {
    pub fn position(&self) -> [i32; 2] {
        [self.slots[0], self.slots[1]]
    }

    pub fn size(&self) -> [i32; 2] {
        [self.slots[2], self.slots[3]]
    }

    pub fn border_size(&self) -> [i32; 2] {
        [self.slots[4], self.slots[5]]
    }

    pub fn cursor(&self) -> [i32; 2] {
        [self.slots[6], self.slots[7]]
    }

    pub fn orientation(&self) -> i32 {
        self.slots[8]
    }

    pub fn flags(&self) -> u8 {
        self.slots[9].to_le_bytes()[0]
    }

    pub fn is_multiline(&self) -> bool {
        self.slots[10].to_le_bytes()[0] != 0
    }
}

impl Default for EditBox {
    fn default() -> Self {
        let mut slots = [0; 12];
        slots[9] = 6;
        slots[10] = 1;
        Self {
            font: b"null_font".to_vec(),
            texture: Vec::new(),
            text: Vec::new(),
            slots,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scrollbar {
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub border_size: [i32; 2],
    pub slots: [u32; 8],
    pub min_value_set: bool,
    pub horizontal: i32,
    pub scroll_speed: i32,
    pub lockable: bool,
    pub drag: bool,
    pub use_range_limit: bool,
    pub range_limit_min: i64,
    pub range_limit_max: i64,
    pub slider: Vec<u8>,
    pub track: Vec<u8>,
    pub right_button: Vec<u8>,
    pub left_button: Vec<u8>,
    pub range_limit_min_icon: Vec<u8>,
    pub range_limit_max_icon: Vec<u8>,
    pub parent: Vec<u8>,
    pub listed: i32,
}

pub const SCROLLBAR_LISTED: [u32; 12] = [
    0x8B, 0x8A, 0x89, 0x88, 0x4C, 0x8D, 0x2F, 0x86, 0x96, 0x95, 0x97, 0x98,
];

impl Scrollbar {
    pub fn slider_pos_shift(&self) -> [i32; 2] {
        [self.slots[0] as i32, self.slots[1] as i32]
    }

    pub fn max_value(&self) -> f32 {
        f32::from_bits(self.slots[2])
    }

    pub fn min_value(&self) -> Option<f32> {
        self.min_value_set.then(|| f32::from_bits(self.slots[3]))
    }

    pub fn step_size(&self) -> f32 {
        f32::from_bits(self.slots[4])
    }

    pub fn start_value(&self) -> f32 {
        f32::from_bits(self.slots[5])
    }

    pub fn priority(&self) -> f32 {
        f32::from_bits(self.slots[6])
    }
}

impl Default for Scrollbar {
    fn default() -> Self {
        Self {
            position: [0; 2],
            size: [0; 2],
            border_size: [0; 2],
            slots: [0; 8],
            min_value_set: false,
            horizontal: 0,
            scroll_speed: 1,
            lockable: false,
            drag: true,
            use_range_limit: false,
            range_limit_min: 0,
            range_limit_max: 0,
            slider: Vec::new(),
            track: Vec::new(),
            right_button: Vec::new(),
            left_button: Vec::new(),
            range_limit_min_icon: Vec::new(),
            range_limit_max_icon: Vec::new(),
            parent: Vec::new(),
            listed: 12,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtendedScrollbar {
    pub position: Position,
    pub size: Size,
    pub tile_size: Size,
    pub orientation: Orientation,
    pub origo: Orientation,
    pub max_value: i64,
    pub min_value: i64,
    pub step_size: i64,
    pub start_value: i64,
    pub smooth_scrolling: i64,
    pub horizontal: bool,
    pub lockable: bool,
    pub drag: bool,
    pub click_only: bool,
    pub set_track_frame_on_change: bool,
    pub background: Option<Box<Element>>,
    pub track: Option<Box<Element>>,
    pub slider: Option<Box<Element>>,
    pub increase_button: Option<Box<Element>>,
    pub decrease_button: Option<Box<Element>>,
}

impl Default for ExtendedScrollbar {
    fn default() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            tile_size: Size::default(),
            orientation: Orientation::UpperLeft,
            origo: Orientation::UpperLeft,
            max_value: 0,
            min_value: 0,
            step_size: 100_000,
            start_value: 0,
            smooth_scrolling: 100_000,
            horizontal: false,
            lockable: false,
            drag: true,
            click_only: false,
            set_track_frame_on_change: false,
            background: None,
            track: None,
            slider: None,
            increase_button: None,
            decrease_button: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropDown {
    pub position: Position,
    pub clipping: bool,
    pub expanded_on_top: bool,
    pub hide_header_on_expand: bool,
    pub contract_on_leave: bool,
    pub switch_frame_on_expand: Option<bool>,
    pub expand_button: Option<Box<Element>>,
    pub expanded_window: Option<Box<Element>>,
    pub inner: Box<Element>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browser {
    pub slots: [i32; 4],
    pub sprite: Vec<u8>,
}

impl Browser {
    pub fn position(&self) -> [i32; 2] {
        [self.slots[0], self.slots[1]]
    }

    pub fn size(&self) -> [i32; 2] {
        [self.slots[2], self.slots[3]]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowExtent {
    Pixels(i32),
    MaxResolution { less: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub position: [i32; 2],
    pub show_position: [i32; 2],
    pub hide_position: [i32; 2],
    pub size: [WindowExtent; 2],
    pub orientation: Orientation,
    pub animation_type: i32,
    pub animation_time: i32,
    pub moveable: i32,
    pub click_to_front: bool,
    pub full_screen: bool,
    pub priority: Option<f32>,
    pub parent: Vec<u8>,
    pub background: Vec<u8>,
    pub vertical_scrollbar: Vec<u8>,
    pub horizontal_scrollbar: Vec<u8>,
    pub horizontal_border: Vec<u8>,
    pub vertical_border: Vec<u8>,
    pub up_sound: Vec<u8>,
    pub down_sound: Vec<u8>,
    pub unlisted: Vec<usize>,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            position: [0; 2],
            show_position: [0; 2],
            hide_position: [0; 2],
            size: [WindowExtent::Pixels(0); 2],
            orientation: Orientation::UpperLeft,
            animation_type: 1,
            animation_time: 10,
            moveable: 1,
            click_to_front: false,
            full_screen: false,
            priority: None,
            parent: Vec::new(),
            background: Vec::new(),
            vertical_scrollbar: Vec::new(),
            horizontal_scrollbar: Vec::new(),
            horizontal_border: Vec::new(),
            vertical_border: Vec::new(),
            up_sound: Vec::new(),
            down_sound: Vec::new(),
            unlisted: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSprite {
    pub font: Vec<u8>,
    pub texture: Vec<u8>,
    pub text: Vec<u8>,
    pub position: [i32; 2],
    pub border_size: [i32; 2],
    pub format: i32,
    pub max_width: i32,
    pub max_height: i32,
    pub fixed_size: bool,
    pub orientation: Orientation,
    pub text_blocks: Vec<TextBlock>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fields {
    Container(Box<Container>),
    Icon(Icon),
    Button(Button),
    TextBox(TextBox),
    Grid(Grid),
    List(List),
    PositionType(PositionType),
    ListBox(ListBox),
    EditBox(EditBox),
    Scrollbar(Scrollbar),
    ExtendedScrollbar(ExtendedScrollbar),
    DropDown(DropDown),
    Browser(Browser),
    Window(Window),
    TextSprite(TextSprite),
    Other,
}

impl Fields {
    fn icon_mut(&mut self) -> Option<&mut Icon> {
        match self {
            Self::Icon(icon) => Some(icon),
            _ => None,
        }
    }

    fn button_mut(&mut self) -> Option<&mut Button> {
        match self {
            Self::Button(button) => Some(button),
            _ => None,
        }
    }

    fn text_box_mut(&mut self) -> Option<&mut TextBox> {
        match self {
            Self::TextBox(text_box) => Some(text_box),
            _ => None,
        }
    }

    fn grid_mut(&mut self) -> Option<&mut Grid> {
        match self {
            Self::Grid(grid) => Some(grid),
            _ => None,
        }
    }

    fn list_mut(&mut self) -> Option<&mut List> {
        match self {
            Self::List(list) => Some(list),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: Kind,
    pub spelling: String,
    pub path: String,
    name: Vec<u8>,
    pub hide: bool,
    pub common: Common,
    pub fields: Fields,
    pub children: Vec<Element>,
    refusals: Vec<Refusal>,
    node: usize,
}

impl Element {
    fn placeholder() -> Self {
        Self {
            kind: Kind::Container,
            spelling: String::new(),
            path: String::new(),
            name: Vec::new(),
            hide: false,
            common: Common::default(),
            fields: Fields::Other,
            children: Vec::new(),
            refusals: Vec::new(),
            node: 0,
        }
    }

    pub fn name(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.name)
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    pub fn is_named(&self, name: impl AsRef<[u8]>) -> bool {
        self.name.eq_ignore_ascii_case(name.as_ref())
    }

    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }

    pub fn nested(&self) -> Vec<&Element> {
        let mut out: Vec<&Element> = Vec::new();
        match &self.fields {
            Fields::Container(container) => out.extend(&container.backgrounds),
            Fields::Grid(grid) => out.extend(grid.background.as_deref()),
            Fields::ExtendedScrollbar(bar) => out.extend(
                [
                    &bar.background,
                    &bar.track,
                    &bar.slider,
                    &bar.increase_button,
                    &bar.decrease_button,
                ]
                .into_iter()
                .filter_map(|part| part.as_deref()),
            ),
            Fields::DropDown(drop_down) => {
                out.extend(drop_down.expand_button.as_deref());
                out.extend(drop_down.expanded_window.as_deref());
                out.push(&drop_down.inner);
            }
            _ => {}
        }
        out.extend(&self.children);
        out
    }

    fn nested_mut(&mut self) -> Vec<&mut Element> {
        let Self {
            fields, children, ..
        } = self;
        let mut out: Vec<&mut Element> = Vec::new();
        match fields {
            Fields::Container(container) => out.extend(container.backgrounds.iter_mut()),
            Fields::Grid(grid) => out.extend(grid.background.as_deref_mut()),
            Fields::ExtendedScrollbar(bar) => out.extend(
                [
                    &mut bar.background,
                    &mut bar.track,
                    &mut bar.slider,
                    &mut bar.increase_button,
                    &mut bar.decrease_button,
                ]
                .into_iter()
                .filter_map(|part| part.as_deref_mut()),
            ),
            Fields::DropDown(drop_down) => {
                out.extend(drop_down.expand_button.as_deref_mut());
                out.extend(drop_down.expanded_window.as_deref_mut());
                out.push(&mut drop_down.inner);
            }
            _ => {}
        }
        out.extend(children.iter_mut());
        out
    }

    pub fn subtree_refusals(&self) -> Vec<&Refusal> {
        let mut out: Vec<&Refusal> = Vec::new();
        let mut stack = vec![self];
        while let Some(element) = stack.pop() {
            for refusal in &element.refusals {
                if !out.contains(&refusal) {
                    out.push(refusal);
                }
            }
            stack.extend(element.nested().into_iter().rev());
        }
        out
    }

    pub fn find(&self, kind: Kind, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        if kind == Kind::PositionType {
            return self.find_instance(name);
        }
        let matches = |child: &&Element| child.kind == kind && child.is_named(name);
        if matches!(self.kind, Kind::Window | Kind::Scrollbar) {
            self.children.iter().rev().find(matches)
        } else {
            self.children.iter().find(matches)
        }
    }

    pub fn find_instance(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        self.children
            .iter()
            .find(|child| child.kind == Kind::PositionType && child.name == name)
    }

    pub fn find_recursive(&self, kind: Kind, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        let mut stack = vec![self];
        while let Some(window) = stack.pop() {
            if let Some(found) = window.find(kind, name) {
                return Some(found);
            }
            stack.extend(
                window
                    .children
                    .iter()
                    .rev()
                    .filter(|child| child.kind == Kind::Container),
            );
        }
        None
    }

    pub fn find_list_recursive(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        let mut stack = vec![self];
        while let Some(window) = stack.pop() {
            if let Some(found) = window.find(Kind::List, name) {
                return Some(found);
            }
            stack.extend(
                window
                    .children
                    .iter()
                    .rev()
                    .filter(|child| child.kind == Kind::Container),
            );
        }
        None
    }

    pub fn child_named(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        if matches!(self.kind, Kind::Window | Kind::Scrollbar) {
            self.children
                .iter()
                .rev()
                .find(|child| child.is_named(name))
        } else {
            self.children.iter().find(|child| child.is_named(name))
        }
    }

    pub fn path_named(&self, path: &str) -> Option<&Element> {
        if path.is_empty() {
            return Some(self);
        }
        let mut current = self;
        for name in path.split('/') {
            current = current.nested_named(name.as_bytes())?;
        }
        Some(current)
    }

    fn nested_named(&self, name: &[u8]) -> Option<&Element> {
        let last = matches!(self.kind, Kind::Window | Kind::Scrollbar);
        let extras = || match &self.fields {
            Fields::Container(container) => {
                if last {
                    container
                        .backgrounds
                        .iter()
                        .rev()
                        .find(|item| item.is_named(name))
                } else {
                    container
                        .backgrounds
                        .iter()
                        .find(|item| item.is_named(name))
                }
            }
            Fields::Grid(grid) => grid
                .background
                .as_deref()
                .filter(|item| item.is_named(name)),
            Fields::ExtendedScrollbar(bar) => {
                let parts = [
                    bar.background.as_deref(),
                    bar.track.as_deref(),
                    bar.slider.as_deref(),
                    bar.increase_button.as_deref(),
                    bar.decrease_button.as_deref(),
                ];
                if last {
                    parts
                        .into_iter()
                        .rev()
                        .flatten()
                        .find(|item| item.is_named(name))
                } else {
                    parts.into_iter().flatten().find(|item| item.is_named(name))
                }
            }
            Fields::DropDown(drop_down) => {
                let parts = [
                    drop_down.expand_button.as_deref(),
                    drop_down.expanded_window.as_deref(),
                    Some(&drop_down.inner),
                ];
                if last {
                    parts
                        .into_iter()
                        .rev()
                        .flatten()
                        .find(|item| item.is_named(name))
                } else {
                    parts.into_iter().flatten().find(|item| item.is_named(name))
                }
            }
            _ => None,
        };
        if last {
            self.child_named(name).or_else(extras)
        } else {
            extras().or_else(|| self.child_named(name))
        }
    }

    pub fn container(&self) -> Option<&Container> {
        match &self.fields {
            Fields::Container(fields) => Some(fields.as_ref()),
            _ => None,
        }
    }

    pub fn icon(&self) -> Option<&Icon> {
        match &self.fields {
            Fields::Icon(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn window(&self) -> Option<&Window> {
        match &self.fields {
            Fields::Window(window) => Some(window),
            _ => None,
        }
    }

    pub fn button(&self) -> Option<&Button> {
        match &self.fields {
            Fields::Button(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn text_box(&self) -> Option<&TextBox> {
        match &self.fields {
            Fields::TextBox(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn grid(&self) -> Option<&Grid> {
        match &self.fields {
            Fields::Grid(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&List> {
        match &self.fields {
            Fields::List(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn position_type(&self) -> Option<&PositionType> {
        match &self.fields {
            Fields::PositionType(fields) => Some(fields),
            _ => None,
        }
    }
}

impl Drop for Element {
    fn drop(&mut self) {
        fn take(element: &mut Element) -> Vec<Element> {
            let mut out = std::mem::take(&mut element.children);
            match &mut element.fields {
                Fields::Container(container) => out.append(&mut container.backgrounds),
                Fields::Grid(grid) => out.extend(grid.background.take().map(|boxed| *boxed)),
                Fields::ExtendedScrollbar(bar) => out.extend(
                    [
                        bar.background.take(),
                        bar.track.take(),
                        bar.slider.take(),
                        bar.increase_button.take(),
                        bar.decrease_button.take(),
                    ]
                    .into_iter()
                    .flatten()
                    .map(|boxed| *boxed),
                ),
                Fields::DropDown(drop_down) => {
                    out.extend(drop_down.expand_button.take().map(|boxed| *boxed));
                    out.extend(drop_down.expanded_window.take().map(|boxed| *boxed));
                    out.append(&mut drop_down.inner.children);
                }
                _ => {}
            }
            out
        }
        let mut stack = take(self);
        while let Some(mut element) = stack.pop() {
            stack.append(&mut take(&mut element));
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gui {
    templates: Vec<Element>,
    default_sounds: DefaultSounds,
    registered: BTreeMap<Vec<u8>, usize>,
    refusals: Vec<Refusal>,
}

impl Gui {
    pub fn template(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        let lowered;
        let key = if name.iter().any(u8::is_ascii_uppercase) {
            lowered = name.to_ascii_lowercase();
            lowered.as_slice()
        } else {
            name
        };
        self.registered
            .get(key)
            .map(|index| &self.templates[*index])
    }

    pub fn templates(&self) -> &[Element] {
        &self.templates
    }

    pub fn text_box_named(&self, name: &[u8]) -> bool {
        let mut stack: Vec<&Element> = self.templates.iter().collect();
        while let Some(element) = stack.pop() {
            if element.kind == Kind::TextSprite && element.name_bytes() == name {
                return true;
            }
            stack.extend(element.nested());
        }
        false
    }

    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    unnamed: i32,
}

pub fn read_at(bytes: &[u8], viewport: [i32; 2]) -> Gui {
    let mut gui = Context::new().read(bytes);
    inherit::pass(&mut [&mut gui], &mut inherit::Viewport::new(Some(viewport)));
    gui
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, bytes: &[u8]) -> Gui {
        let mut reader = reader::Reader::new(
            content_reader_script::native::lex(bytes, content_reader_script::native::Mode::KeepMark),
            self.unnamed,
        );
        let blocks = reader.file();
        self.unnamed = reader.unnamed;
        finish(reader, blocks)
    }

    pub fn read_all<B: AsRef<[u8]> + Sync>(&mut self, files: &[B]) -> Vec<Gui> {
        let read_from = |bytes: &[u8], base: i32| {
            let mut context = Context { unnamed: base };
            let gui = context.read(bytes);
            (gui, context.unnamed.wrapping_sub(base))
        };
        let first = content_reader_layers::parallel::map(files, |bytes| read_from(bytes.as_ref(), 0));
        let mut again = Vec::new();
        let mut guis = Vec::with_capacity(first.len());
        for (index, (gui, named)) in first.into_iter().enumerate() {
            if named != 0 && self.unnamed != 0 {
                again.push((index, self.unnamed));
            }
            self.unnamed = self.unnamed.wrapping_add(named);
            guis.push(gui);
        }
        let reread = content_reader_layers::parallel::map(&again, |&(index, base)| {
            read_from(files[index].as_ref(), base).0
        });
        for ((index, _), gui) in again.into_iter().zip(reread) {
            guis[index] = gui;
        }
        guis
    }
}

pub fn read(bytes: &[u8]) -> Gui {
    let mut gui = Context::new().read(bytes);
    inherit::pass(&mut [&mut gui], &mut inherit::Viewport::new(None));
    gui
}

pub const LOAD_SCREEN: &str = "interface/load_screen.gui";

pub fn read_layers(layers: &ResolvedLayers) -> io::Result<Guis> {
    read_layers_with(layers, None)
}

pub fn read_layers_at(layers: &ResolvedLayers, viewport: [i32; 2]) -> io::Result<std::sync::Arc<Guis>> {
    read_shared_layers(layers, Some(viewport))
}

pub fn read_shared_layers(layers: &ResolvedLayers, viewport: Option<[i32; 2]>) -> io::Result<std::sync::Arc<Guis>> {
    let _loading = (!layers.is_generation_scoped()).then(|| layers.loading());
    let key = viewport.map_or_else(Vec::new, |size| size.map(i32::to_le_bytes).concat());
    layers.compatible_product("inherited GUI layouts", 1, &key,
        |guis: &Guis| viewport.map_or(!guis.needs_viewport, |size| guis.fits_viewport(size)),
        |layers| read_layers_with(layers, viewport),
    )
}

fn read_layers_with(layers: &ResolvedLayers, viewport: Option<[i32; 2]>) -> io::Result<Guis> {
    let definitions = layers.product("GUI definitions before inheritance", 1, &[], read_layer_definitions)?;
    let mut files = definitions.as_ref().clone();
    let mut tested = inherit::Viewport::new(viewport);
    if let Some((path, gui)) = files.first_mut()
        && path == LOAD_SCREEN
    {
        inherit::pass(&mut [gui], &mut tested);
    }
    inherit::pass(
        &mut files.iter_mut().map(|(_, gui)| gui).collect::<Vec<_>>(),
        &mut tested,
    );
    let mut guis = Guis::new(files);
    guis.needs_viewport = tested.needed;
    if viewport.is_some() || !tested.needed {
        guis.viewport = Some(tested.comparisons);
    }
    Ok(guis)
}

fn read_layer_definitions(layers: &ResolvedLayers) -> io::Result<Vec<(String, Gui)>> {
    let mut context = Context::new();
    let mut files: Vec<(String, Gui)> = Vec::new();
    let listed = layers.listed_layer_paths(b"interface/")?;
    let label = |path: &[u8]| String::from_utf8_lossy(path).into_owned();
    if let Some(bytes) = content_reader_layers::content::open_request(layers, LOAD_SCREEN.as_bytes())? {
        files.push((LOAD_SCREEN.to_owned(), context.read(&bytes)));

    }
    let paths: Vec<Vec<u8>> =
        content_reader_defines::defines::load_order_paths(b"interface/", b".gui", listed.iter().map(Vec::as_slice))
            .into_iter()
            .filter(|path| path != LOAD_SCREEN.as_bytes())
            .collect();
    let texts = layers
        .read_paths(&paths)
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
    for (path, gui) in paths.iter().zip(context.read_all(&texts)) {
        files.push((label(path), gui));
    }
    Ok(files)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guis {
    pub files: Vec<(String, Gui)>,
    registered: Vec<(Vec<u8>, usize, usize)>,
    viewport: Option<inherit::Comparisons>,
    needs_viewport: bool,
}

impl Guis {
    pub fn fits_viewport(&self, viewport: [i32; 2]) -> bool {
        self.viewport.as_ref().is_some_and(|tested| tested.matches(viewport))
    }

    pub fn default_sounds(&self) -> DefaultSounds {
        let mut sounds = DefaultSounds::default();
        for (_, gui) in &self.files {
            let set = &gui.default_sounds;
            for (into, from) in [
                (&mut sounds.click, &set.click),
                (&mut sounds.over, &set.over),
                (&mut sounds.down, &set.down),
                (&mut sounds.up, &set.up),
            ] {
                if from.is_some() {
                    into.clone_from(from);
                }
            }
        }
        sounds
    }

    pub fn new(files: Vec<(String, Gui)>) -> Self {
        let mut registered = BTreeMap::new();
        for (file, (_, gui)) in files.iter().enumerate() {
            for (name, &template) in &gui.registered {
                registered.insert(name.clone(), (file, template));
            }
        }
        Self {
            files,
            viewport: None,
            needs_viewport: false,
            registered: registered
                .into_iter()
                .map(|(name, (file, template))| (name, file, template))
                .collect(),
        }
    }

    pub fn template(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        let lowercase = !name.iter().any(u8::is_ascii_uppercase);
        let index = self
            .registered
            .binary_search_by(|(key, ..)| {
                if lowercase {
                    key.as_slice().cmp(name)
                } else {
                    key.iter()
                        .copied()
                        .cmp(name.iter().map(u8::to_ascii_lowercase))
                }
            })
            .ok()?;
        let (_, file, template) = &self.registered[index];
        Some(&self.files[*file].1.templates[*template])
    }

    pub fn text_box_named(&self, name: &[u8]) -> bool {
        self.files.iter().any(|(_, gui)| gui.text_box_named(name))
    }

    pub fn refusals(&self) -> Vec<(&str, &Refusal)> {
        self.files
            .iter()
            .flat_map(|(path, gui)| {
                gui.refusals
                    .iter()
                    .map(move |refusal| (path.as_str(), refusal))
            })
            .collect()
    }
}

fn batch_order(count: usize) -> Vec<usize> {
    let mut inserted = vec![false; count];
    let mut order = Vec::with_capacity(count);
    let mut step = count.div_ceil(2);
    let mut start = count;
    while step > 0 {
        start /= 2;
        let mut index = start;
        while index < count {
            if !std::mem::replace(&mut inserted[index], true) {
                order.push(index);
            }
            index += step;
        }
        step = if step < 3 {
            step >> 1
        } else {
            step.div_ceil(2)
        };
    }
    order.extend((0..count).filter(|index| !inserted[*index]));
    order
}

fn finish(reader: reader::Reader, blocks: Vec<Vec<Element>>) -> Gui {
    let mut paths: Vec<String> = Vec::with_capacity(reader.labels.len());
    for label in &reader.labels {
        let path = match label.parent {
            Some(parent) => format!("{}.{}", paths[parent], label.text),
            None => label.text.clone(),
        };
        paths.push(path);
    }
    let mut templates = Vec::new();
    let mut registered = BTreeMap::new();
    for mut block in blocks {
        let first = templates.len();
        let mut sorted: Vec<usize> = (0..block.len()).collect();
        sorted.sort_by(|a, b| block[*a].name.cmp(&block[*b].name));
        for position in batch_order(sorted.len()) {
            let index = sorted[position];
            registered.insert(block[index].name.to_ascii_lowercase(), first + index);
        }
        templates.append(&mut block);
    }
    let mut stack: Vec<&mut Element> = templates.iter_mut().collect();
    while let Some(element) = stack.pop() {
        element.path = paths[element.node].clone();
        for refusal in &mut element.refusals {
            refusal.path = paths[refusal.node].clone();
        }
        stack.extend(element.nested_mut());
    }
    let mut refusals = reader.file;
    if let Some(poison) = reader.poison
        && !refusals.contains(&poison)
    {
        refusals.push(poison);
    }
    for refusal in &mut refusals {
        refusal.path = paths[refusal.node].clone();
    }
    Gui {
        templates,
        registered,
        refusals,
        default_sounds: reader.default_sounds,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultSounds {
    pub click: Option<Vec<u8>>,
    pub over: Option<Vec<u8>>,
    pub down: Option<Vec<u8>>,
    pub up: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonSounds {
    pub over: Option<Vec<u8>>,
    pub down: Option<Vec<u8>>,
    pub up: Option<Vec<u8>>,
    pub click: Option<Vec<u8>>,
    pub uncheck: Option<Vec<u8>>,
}

impl Button {
    pub fn sounds(&self, sprite_click: &[u8], defaults: &DefaultSounds) -> ButtonSounds {
        let own = |sound: &[u8]| (!sound.is_empty()).then(|| sound.to_vec());
        let fallback = |sound: &Option<Vec<u8>>| {
            sound
                .clone()
                .filter(|sound| !sound.is_empty() && !self.no_click_sound)
        };
        let down = own(&self.down_sound).or_else(|| fallback(&defaults.down));
        let click = (!self.no_click_sound)
            .then(|| {
                own(&self.click_sound)
                    .or_else(|| own(sprite_click))
                    .or_else(|| fallback(&defaults.click))
            })
            .flatten();
        ButtonSounds {
            over: own(&self.over_sound).or_else(|| fallback(&defaults.over)),
            up: own(&self.up_sound).or_else(|| fallback(&defaults.up)),
            uncheck: own(&self.up_sound).or_else(|| click.clone()),
            click,
            down,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn the_last_registering_file_wins_without_hiding_other_templates() {
        content_reader_script::test_game::enter();
        let guis = Guis::new(vec![
            (
                "first.gui".into(),
                read(
                    br#"guiTypes={
                containerWindowType={name=Shared size={width=11 height=12}}
                containerWindowType={name=first_only}
            }"#,
                ),
            ),
            (
                "last.gui".into(),
                read(
                    br#"guiTypes={
                containerWindowType={name=sHARED size={width=21 height=22}}
                containerWindowType={name=last_only}
            }"#,
                ),
            ),
            ("empty.gui".into(), Gui::default()),
        ]);
        for name in ["shared", "SHARED", "sHaReD"] {
            assert!(std::ptr::eq(
                guis.template(name).unwrap(),
                guis.files[1].1.template(name).unwrap(),
            ));
        }
        assert!(guis.template("FIRST_ONLY").is_some());
        assert!(guis.template("LAST_ONLY").is_some());
        assert!(guis.template("missing").is_none());
        let cloned = guis.clone();
        assert_eq!(cloned, guis);
        assert!(std::ptr::eq(
            cloned.template("shared").unwrap(),
            cloned.files[1].1.template("shared").unwrap(),
        ));
    }
}
