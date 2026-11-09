//! The game's reading of `.gui` files, as typed element trees.
//!
//! [`read_layers`] reads every `.gui` file of the content stack in the game's load order with
//! one [`Context`]. [`Context::read`] (or [`read`] for a first file) reads one file through the
//! shared lexer and token reader ([`crate::parser::native`]). It keeps every element the game
//! builds, with its authored name or generated `unnamed_gui_<n>`, each stored field with its
//! default, and its kept children in declaration order. Unknown keys are logged and their
//! blocks skipped. A field reader that does not consume its block leaves it to be read as
//! further keys; a key without an operator passes the previous value to its parser. [`Refusal`]
//! names what the game itself does not define: an invalid read, an endless loop or a crash,
//! with the element's full path and key. A consumer draws an element only when nothing it
//! depends on is refused ([`Element::refusals`], [`Element::subtree_refusals`]).
//!
//! # Tokens
//!
//! [`crate::parser::native::lex`] uses mode 0. Token identity decides dispatch: an unquoted
//! word starting with a digit or `-` is a number (0xC) before lookup. Other unquoted text is
//! looked up as a static keyword up to its first NUL after folding ASCII A-Z; failing that,
//! text containing only digits, `-` and `.` is a number, and other text is a string (0xF).
//! Quoted strings are always 0xF. Quoted keys therefore do not dispatch. Value readers using
//! keyword identity accept only unquoted text; string, boolean and `%i`, `%u`, `%x`, `%f`
//! readers accept either form, taking text up to its first NUL.
//!
//! # Key loops
//!
//! Elements, `guiTypes` and the file share one persistent loop. It peeks, then consumes `}` or
//! end-of-stream to finish the level. Otherwise it reads key/operator/value slots and
//! dispatches the key's identity. A key starting with `@` defines a variable, and the following
//! token becomes the next key. After a key, a token other than `=`, `<` or `>` is lost; the
//! operator and value slots retain the previous key's values. After an operator, a peeked `}`
//! remains unread and leaves an empty value. A value starting with `@` is read and resolved
//! only into the value slot; other values are read normally. Block skipping inspects the
//! The lexer's current token and skips only when that token is `{`. Unknown keys record an error,
//! skip a block value and continue. Nesting has no chosen limit: each reader follows the game's
//! nesting on its own stack.
//!
//! # Script variables
//!
//! A variable definition appends the name's FNV-1a hash to the file's initially empty table
//! unless already present, in which case it reads nothing more. For a new name, a token other
//! than `=` is lost and stores nothing. After `=`, a value starting with `@` is resolved
//! without being consumed, so the loop takes it again as its next key; other values are read
//! and stored as tokens. Values resolve through the one [`crate::parser::native::Reader`]
//! table. Every sub-reader writes its resolved token back as the lexer's current token.
//!
//! # Names and lookups
//!
//! Every `name` assignment replaces the previous name. An element still unnamed after reading
//! takes `unnamed_gui_` followed by a global decimal counter. That counter starts at 0, never
//! resets and advances once per generated name across all files ([`Context`]). A `background`
//! (0x9C) remains unnamed and takes no number. Containers read each child completely before
//! registering it in one name tree shared by all kinds, comparing with each byte lowered. A
//! later duplicate child is destroyed, so the first wins. `windowType` and `scrollbarType`
//! replace duplicates, so their last child wins. Kept children attach in declaration order.
//! Kind-specific lookup searches only that kind's array. Templates use the shared registry into
//! which every file's `guiTypes` insert, with later inserts replacing earlier ones.
//!
//! # After loading
//!
//! The post-load pass runs after `load_screen.gui` and again after every file. Each template
//! visits its children with the parent's name tree. Buttons copy an empty sprite from the
//! `parent` chain and parse their orientation text. Scrollbars copy unset fields from their
//! `parent` chain. List boxes never finish when `parent` is set. Walks that hang or read
//! through a null or the wrong element kind are refused according to that behavior.

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

pub use windows::{kept_container_window, window_button_sprite, window_button_tooltip, window_texts};

use crate::parser::native::Token;
pub use reader::{
    Axis as PointAxis, Case, PersistentKey, bool_of, fnv64 as tooltip_hash, persistent_keys,
};

use crate::ResolvedLayers;

/// Why part of a document is not read as the game reads it: the element's full path, the key,
/// and what the game itself does there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The element's path from its template: `window.child.grandchild`; empty for the file's own
    /// level.
    pub path: String,
    /// The key as authored; empty for the element itself.
    pub key: String,
    pub reason: String,
    pub cause: Cause,
    /// The element the refusal was recorded on, whose path is known once the file is read.
    node: usize,
}

/// What a refusal records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// A field the game stores from an invalid or undefined read, or an element whose post-load
    /// walk never ends or crashes: the field or element is refused.
    Unread,
    /// A token walk the game itself does not define (a comparison with stack bytes, a loop
    /// that never ends): every element that ends after it is refused, since where the game then
    /// reads is undefined.
    Unaligned,
    /// What the game did depends on a value of the reading client's session the caller did not
    /// give (the GUI root's extent when the file was read, [`read_layers_at`]): the element is
    /// refused as missing input, not as a read the game does not define.
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

/// An element kind the game builds from a `.gui` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `containerWindowType` (token 0x264).
    Container,
    /// `iconType` (0xB6).
    Icon,
    /// `buttonType` (0x271).
    Button,
    /// `checkboxType` (0xC4): a button kept in its own array and drawn as a checkbox.
    Checkbox,
    /// `guiButtonType` (0x7F): a button a `windowType`, a scrollbar or `guiTypes` builds.
    GuiButton,
    /// `multiSpriteButtonType` (0xB4): a button a `windowType` builds.
    MultiSpriteButton,
    /// `instantTextBoxType` (0x120).
    TextBox,
    /// `gridBoxType` (0x275).
    Grid,
    /// `OverlappingElementsBoxType` (0x14E).
    List,
    /// `positionType` (0xB2), which draws nothing (no instance factory).
    PositionType,
    /// `listBoxType` (0xA9), list box.
    ListBox,
    /// `smoothListboxType` (0x1FB), the same list box kept in its own array.
    SmoothListBox,
    /// `textBoxType` (0x85), text box.
    TextSprite,
    /// `editBoxType` (0xAA).
    EditBox,
    /// `scrollbarType` (0x8C), scrollbar.
    Scrollbar,
    /// `extendedScrollbarType` (0x265), extended scrollbar.
    ExtendedScrollbar,
    /// `dropDownBoxType` (0x26C), drop-down.
    DropDown,
    /// `browserType` (0x287), browser.
    Browser,
    /// `curveGraphType` (0x288), curve graph.
    CurveGraph,
    /// `windowType` (0x9B), window.
    Window,
    /// A `guiTypes` kind no container or window builds (`radioButtonGroupType`,
    /// `scrollbargroupType`, `layout`, `dropDownMenuType`), by its keyword: kept for its name and
    /// refused, its block read to its end.
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

    /// The container lookup that binds a child of this kind by name, over this kind's
    /// own array only.
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

/// An orientation (anchor) by meaning. Container and grid keyword readers store these nine
/// values as 0 to 8 in order. The text reader knows seven: it stores `center_up` and
/// `center_down` as 5 and 6 and has no `center_left` or `center_right`.
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

    /// The orientation a container's or grid's native value 0 to 8 stands for.
    pub fn from_value(value: i32) -> Option<Self> {
        Self::NAMES
            .get(usize::try_from(value).ok()?)
            .map(|(_, own)| *own)
    }

    /// The text reader's stored value: `upper_left` 0, `upper_right` 1, `lower_left` 2,
    /// `lower_right` 3, `center` 4, `center_up` 5, `center_down` 6.
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

    /// The keyword token among the nine orientations.
    fn from_keyword(token: &Token) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(name, _)| token.is_keyword(name))
            .map(|(_, own)| *own)
    }

    /// Text lowered and compared exactly with its seven names. Anything else (`centre`,
    /// `center_left` and `center_right` included) is `upper_left`, with no error.
    fn parse_gui(text: &[u8]) -> Self {
        Self::NAMES
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, own)| *own)
            .filter(|own| !matches!(own, Self::CenterLeft | Self::CenterRight))
            .unwrap_or_default()
    }
}

/// A text box's or button's horizontal alignment: `left` 0, `right` 1, `center` 2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextFormat {
    #[default]
    Left,
    Right,
    Center,
}

/// A text box's or button's `vertical_alignment`: `top` 0, `center` 1, `bottom` 2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}

/// An overlapping-elements box's `format`: `center`/`centre` 0, `right` 1, `left` 2 (the
/// constructor's default), `up` (0x6C) 3, `down` (0x31E) 4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListFormat {
    Center,
    Right,
    #[default]
    Left,
    Up,
    Down,
}

/// A grid's `format` by stored value: `UP` 0, `DOWN` 1, `LEFT` 2, `RIGHT` 3, `UPPER_LEFT` 4,
/// `LOWER_LEFT` 5, `UPPER_RIGHT` 6.
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

    /// Placement for a stored value: its own for 0 to 6, the default (`UP`) for any other.
    pub fn from_value(value: i32) -> Self {
        usize::try_from(value)
            .ok()
            .and_then(|index| Self::VALUES.get(index))
            .copied()
            .unwrap_or(Self::Up)
    }

    /// The token's text, compared case-sensitively; other text is logged as invalid and stored
    /// as 0.
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

/// A container's animation or fade type: `none` 0, `accelerated` 1, `decelerated` 2,
/// `smoothstep` 3, `linear` 4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Animation {
    #[default]
    None,
    Accelerated,
    Decelerated,
    Smoothstep,
    Linear,
}

/// One position or size coordinate: a number replaces the value and clears the auto bit. The
/// percentage and position-relative bits are ORed in and never cleared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coordinate {
    pub value: i32,
    /// Written with a trailing `%`: a percentage of the parent.
    pub percent: bool,
    /// Written with `%%` in a size: evaluation subtracts the position.
    pub from_position: bool,
    /// The auto bit: set by `auto` (0x2A2, with the value 0) and by the constructors of the
    /// bounds; a bound whose auto bit is set bounds nothing.
    pub auto: bool,
}

impl Coordinate {
    /// An unassigned bound: the constructors' flags `0x30`.
    pub const UNBOUNDED: Self = Self {
        value: 0,
        percent: false,
        from_position: false,
        auto: true,
    };

    /// A plain number.
    pub fn pixels(value: i32) -> Self {
        Self {
            value,
            ..Self::default()
        }
    }
}

/// The constructors' `step`: 1 by 1, no flags.
const STEP: [Coordinate; 2] = [Coordinate {
    value: 1,
    percent: false,
    from_position: false,
    auto: false,
}; 2];

/// An element's position: two coordinates and the `min` and `max` bounds evaluation clamps to.
/// `step` is read but no evaluator uses it.
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

/// An element's size, with its constructor defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub size: [Coordinate; 2],
    pub min: [Coordinate; 2],
    pub max: [Coordinate; 2],
    /// `step`, unused by size evaluation.
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

/// A margin, initially zero, with one position coordinate per side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub left: Coordinate,
    pub top: Coordinate,
    pub right: Coordinate,
    pub bottom: Coordinate,
}

/// `pdx_tooltip_anchor`: the read flag its key sets, then its own key loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipAnchor {
    /// Whether the key was read.
    pub set: bool,
    /// `x` and `y`, `%i`.
    pub x: i32,
    pub y: i32,
    /// `relative` (1) and `below` (1).
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

/// The common fields each element keeps beyond its name and `hide`. Map icons receive no hit
/// list, so their objects take no pointer input, hover or tooltip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Common {
    /// `tooltip`, `tooltipText`, `pdx_tooltip`: FNV-1a/64 of the value's text, 0 when none.
    pub tooltip: u64,
    /// Authored C-string bytes retained for hash-based localisation.
    pub tooltip_key: Vec<u8>,
    /// `delayedTooltipText`, `pdx_tooltip_delayed`.
    pub delayed_tooltip: u64,
    pub delayed_tooltip_key: Vec<u8>,
    /// `pdx_disabled_tooltip`.
    pub disabled_tooltip: u64,
    pub disabled_tooltip_key: Vec<u8>,
    /// `pdx_disabled_tooltip_delayed`.
    pub disabled_tooltip_delayed: u64,
    pub disabled_tooltip_delayed_key: Vec<u8>,
    /// `hint_tag`.
    pub hint_tag: Vec<u8>,
    pub tooltip_anchor: TooltipAnchor,
    /// `bound_tooltip` or `context_aware_tooltip`, read only while empty.
    pub bound_tooltip: BoundLocalization,
    /// `context_aware_tooltip`: the tooltip needs the view's scoped localizer.
    pub context_aware_tooltip: bool,
}

/// Formatted localization: text holding `token:` anywhere is a token (the text from its seventh
/// byte on). Otherwise, text holding `|` names a formatter before the separator and a token
/// after it; the formatter is looked up in the engine's formatter map. Other text is a
/// localisation key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormattedLocalization {
    Key(Vec<u8>),
    Formatter { formatter: Vec<u8>, token: Vec<u8> },
    Token(Vec<u8>),
}

impl Default for FormattedLocalization {
    fn default() -> Self {
        Self::Key(Vec::new())
    }
}

impl FormattedLocalization {
    pub(crate) fn parse(text: &[u8]) -> Self {
        if let Some(at) = text.windows(6).position(|window| window == b"token:") {
            return Self::Token(text.get(at + 6..).unwrap_or_default().to_vec());
        }
        match text.iter().position(|byte| *byte == b'|') {
            Some(bar) => Self::Formatter {
                formatter: text[..bar].to_vec(),
                token: text[bar + 1..].to_vec(),
            },
            None => Self::Key(text.to_vec()),
        }
    }

    /// Whether the localisation key is empty, as tested by missing-key handling.
    pub fn is_empty_key(&self) -> bool {
        matches!(self, Self::Key(key) if key.is_empty())
    }
}

/// One node of a [`BoundLocalization`] (`0x48` bytes): its key and its named arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundNode {
    /// `localization_key`, the last assignment.
    pub key: FormattedLocalization,
    /// Every other key of the block, in order, duplicates kept.
    pub arguments: Vec<BoundArgument>,
}

/// A named argument of a [`BoundNode`] (`0x58` bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundArgument {
    /// The key's text as lexed.
    pub name: Vec<u8>,
    pub value: BoundValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundValue {
    /// A scalar value.
    Localization(FormattedLocalization),
    /// A block value: the node at this index of [`BoundLocalization::nodes`].
    Node(usize),
}

/// Bound localization nodes in pre-order, root first; empty when unset or when the root lacks a
/// key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundLocalization {
    pub nodes: Vec<BoundNode>,
}

impl BoundLocalization {
    /// Project the shared bound reader, retaining authored argument order and the last
    /// localization key.
    pub fn from_value(value: &crate::parser::Value) -> Self {
        fn formatted(value: &crate::parser::Value) -> FormattedLocalization {
            FormattedLocalization::parse(crate::parser::c_string(
                value.read_string().unwrap_or(b"{"),
            ))
        }
        fn node(value: &crate::parser::Value, result: &mut BoundLocalization) -> usize {
            let at = result.nodes.len();
            result.nodes.push(BoundNode::default());
            if let Some(fields) = value.items() {
                for field in fields {
                    if field.keyword_is("localization_key") {
                        result.nodes[at].key = formatted(&field.value);
                    } else {
                        let value = if field.value.items().is_some() {
                            BoundValue::Node(node(&field.value, result))
                        } else {
                            BoundValue::Localization(formatted(&field.value))
                        };
                        result.nodes[at].arguments.push(BoundArgument {
                            name: crate::parser::c_string(field.key.as_deref().unwrap_or_default())
                                .to_vec(),
                            value,
                        });
                    }
                }
            } else {
                result.nodes[at].key = formatted(value);
            }
            at
        }
        let mut result = Self::default();
        node(value, &mut result);
        if value.items().is_some() && result.nodes[0].key.is_empty_key() {
            result.nodes.clear(); // The root lacks a localization key.
        }
        result
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// A `textblock` of a `textBoxType`. Construction hands the list to the text sprite, which
/// clears its output list for every sprite class: no text block draws.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    /// `color`, read as a color and initially white.
    pub color: [f32; 4],
    /// `font` and `text`.
    pub font: Vec<u8>,
    pub text: Vec<u8>,
    /// `position`: integer point reader into a zeroed local; each assignment also adds the old
    /// position less the new one to `size`.
    pub position: [i32; 2],
    /// `size`: integer point reader into a zeroed local.
    pub size: [i32; 2],
    /// `format`: `left` 0, `right` 1 and `centre` 2 by keyword; anything else leaves it.
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

/// A `containerWindowType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    /// `position`; every assignment also copies itself over `show_position` and
    /// `hide_position`.
    pub position: Position,
    /// `show_position`: the rest position after a show animation, from which negative and `%%`
    /// sizes are measured.
    pub show_position: Position,
    pub hide_position: Position,
    pub size: Size,
    /// Token 0xC2: a keyword among the nine orientations; other values are logged and stored as
    /// `upper_left`.
    pub orientation: Orientation,
    /// Token 0x66, read like `orientation`.
    pub origo: Orientation,
    /// `yes` sets the clip byte to 1 and other text to 0, with the local cleared before
    /// reading. `None` leaves the constructor's -1, which enables clipping exactly when the
    /// window has backgrounds.
    pub clipping: Option<bool>,
    pub margin: Margin,
    pub background_margin: Margin,
    /// Every `background` block, each read as a button, in order.
    pub backgrounds: Vec<Element>,
    /// `scale` (initially 1.0): the content scale.
    pub scale: f32,
    /// `show_animation_type`, and `animation_type` (which sets both).
    pub show_animation: Animation,
    pub hide_animation: Animation,
    /// `fade_type`: anything but `none` starts the window transparent.
    pub fade: Animation,
    /// `fade_time`, milliseconds (the constructor's 1000).
    pub fade_time: i32,
    /// `fade_delay`, milliseconds (0): the fade update waits it out.
    pub fade_delay: i32,
    /// `show_animation_time` and `hide_animation_time`, milliseconds (the constructor's 10 each);
    /// `animation_time` writes both.
    pub show_animation_time: i32,
    pub hide_animation_time: i32,
    /// `moveable` (0): an immovable window snaps to its old target before moving. Its backgrounds
    /// also use this flag to decide whether they can drag it.
    pub moveable: bool,
    /// `autohide_scrollbars` (initially 1): a scrollbar is shown during construction only when this
    /// is `no`.
    pub autohide_scrollbars: bool,
    /// `verticalScrollbar` and `horizontalScrollbar`: named scrollbar types instantiated during
    /// construction.
    pub vertical_scrollbar: Vec<u8>,
    pub horizontal_scrollbar: Vec<u8>,
    /// `click_to_front` and `smooth_scrolling`.
    pub click_to_front: bool,
    pub smooth_scrolling: bool,
    /// `scroll_wheel_factor` (15.0), `vertical_scroll_step` (1.0), `horizontal_scroll_step` (1.0):
    /// `%f`.
    pub scroll_wheel_factor: f32,
    pub vertical_scroll_step: f32,
    pub horizontal_scroll_step: f32,
    /// `drag_scroll`: each button (`left` 1, `middle` 2, `right` 4, `none` 0) ORed in.
    pub drag_scroll: u8,
    /// `closed_position`, `open_position`, `peek_position`, `peek_check_top_left`,
    /// `peek_check_size`: hover movement positions and bounds.
    pub closed_position: Position,
    pub open_position: Position,
    pub peek_position: Position,
    pub peek_check_top_left: Position,
    pub peek_check_size: Size,
    /// The hover flag: set once any of the five hover keys is read, even malformed.
    pub peek: bool,
    /// `upsound`/`show_sound` and `downsound`/`hide_sound`.
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

/// An `iconType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    /// `spriteType`, `quadTextureSprite` and `buttonMesh` all write the same sprite field; empty
    /// when none.
    pub sprite: Vec<u8>,
    /// An integer point read into a zeroed local, with both axes then stored as floats.
    /// Integers past 2^24 round to the nearest float. Construction and drawable building
    /// truncate them back with `cvttss2si` ([`crate::crt::truncate_f32`]), so 2147483647
    /// becomes `i32::MIN` there. `3dposition` (0xAF) starts with a digit, so the lexer treats
    /// it as a number and it never dispatches: the icon stays 2D.
    pub position: [f32; 2],
    pub scale: f32,
    /// The text orientation of each assignment.
    pub orientation: Orientation,
    /// The constructor's 1.
    pub frame: i32,
    pub rotation: f32,
    /// `alwaystransparent` and `allwaystransparent`.
    pub always_transparent: bool,
    /// `centerposition`, the icon's flag-word bit 6: centred on its position.
    pub center_position: bool,
    /// `center_on_anchor`, the icon's flag-word bit 7.
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

/// A button's fields and defaults: a `buttonType`, `checkboxType`, `guiButtonType`,
/// `multiSpriteButtonType`, `background` or a scrollbar's or drop-down's button part.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    /// `spriteType` and `quadTextureSprite`; empty when none. The post-load pass copies a sprite
    /// from the `parent` chain when empty.
    pub sprite: Vec<u8>,
    /// `position` and `size`: integer point reader into the field.
    pub position: [i32; 2],
    pub scale: f32,
    pub rotation: f32,
    /// Initially 0; construction passes it on only when above 0.
    pub frame: i32,
    /// The last `orientation` text (the constructor's `UPPER_LEFT`).
    pub orientation_text: Vec<u8>,
    /// The text orientation, written only by the post-load pass. `None` for a button that pass
    /// never reaches (a container's or grid's `background`), whose field is left uninitialised.
    pub orientation: Option<Orientation>,
    /// `centerposition`.
    pub center_position: bool,
    /// `alwaystransparent` only; `allwaystransparent` is no button key.
    pub always_transparent: bool,
    /// `enabled` (initially 1): `no` disables the button, drawing its Disable state.
    pub enabled: bool,
    /// `font`, `buttonFont` and `origo` all write the same font field.
    pub font: Vec<u8>,
    /// `text` and `buttonText`.
    pub text: Vec<u8>,
    /// The constructor's 2, center.
    pub format: TextFormat,
    /// The constructor's 1, center.
    pub vertical_alignment: VerticalAlignment,
    pub multiline: bool,
    pub size: [i32; 2],
    /// `borderSize`: integer point reader into a zeroed local, both axes copied.
    pub border_size: [i32; 2],
    /// `parent`.
    pub parent: Vec<u8>,
    /// `upSprite`, `downSprite`, `overSprite`, `dissableSprite`: read, and passed to no
    /// constructor.
    pub up_sprite: Vec<u8>,
    pub down_sprite: Vec<u8>,
    pub over_sprite: Vec<u8>,
    pub dissable_sprite: Vec<u8>,
    /// Each `shortcut`, appended.
    pub shortcuts: Vec<Vec<u8>>,
    /// `luaClick`.
    pub lua_click: Vec<u8>,
    /// `callback_type` and `callback_argument`; `web_link` sets the type to `web_link` and its text
    /// as the argument.
    pub callback_type: Vec<u8>,
    pub callback_argument: Vec<u8>,
    /// `upsound`, `downsound`, `oversound`, `clicksound` (exactly `none` clears it and sets
    /// `no_clicksound`).
    pub up_sound: Vec<u8>,
    pub down_sound: Vec<u8>,
    pub over_sound: Vec<u8>,
    pub click_sound: Vec<u8>,
    /// `no_clicksound`.
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

/// An `instantTextBoxType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    /// Empty when the box names none; font lookup with fallback resolves it.
    pub font: Vec<u8>,
    pub position: [i32; 2],
    pub format: TextFormat,
    /// `%u`; 0 when absent, which text object construction makes 200.
    pub max_width: u32,
    pub max_height: u32,
    pub text: Vec<u8>,
    /// The last `orientation` string, parsed in the post-load pass.
    pub orientation: Orientation,
    pub vertical_alignment: VerticalAlignment,
    /// `alwaystransparent` and `allwaystransparent`.
    pub always_transparent: bool,
    /// `borderSize`.
    pub border_size: [i32; 2],
    /// `scrollbarType`: a named scrollbar type the box instantiates, clipping its text.
    pub scrollbar_type: Vec<u8>,
    /// `fixedsize`.
    pub fixed_size: bool,
    /// `truncate` (the constructor's 1).
    pub truncate: bool,
    /// `multiline` (the constructor's 1).
    pub multiline: bool,
    /// `text_color_code`: one byte, 0 for none.
    pub text_color_code: u8,
    /// `ignore_special_chars`.
    pub ignore_special_chars: bool,
    /// `bound_text` or `context_aware_text`, read only while empty. Text refresh shows it only when
    /// `context_aware` is set and the box is shown and queued, through the view's scoped localizer.
    /// Before that, and otherwise, the box shows `text`.
    pub bound_text: BoundLocalization,
    /// `context_aware_text`.
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

/// A `gridBoxType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub position: Position,
    pub size: Size,
    pub slot_size: Size,
    /// The format word as stored: `format`'s value, or what `max_slots`' third component wrote
    /// ([`GridFormat::from_value`]).
    pub format: i32,
    /// The orientation word as stored, the containers' numbering 0 to 8
    /// ([`Orientation::from_value`]), or what `max_slots`' fourth component wrote.
    pub orientation: i32,
    /// `[max_slots_horizontal, max_slots_vertical]`, the constructor's -1 each.
    pub max_slots: [i32; 2],
    /// Initially 1: `add_horizontal`, or the low byte of a fifth `max_slots` component. The integer
    /// component reader writes an `i32` over this byte and following padding; grid construction
    /// copies the byte to the instance's fill-direction flag.
    pub add_horizontal: bool,
    /// `padding`.
    pub padding: Margin,
    /// The first `background`, read as a button.
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

/// An `OverlappingElementsBoxType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct List {
    /// `parent`, retained without a consumer in the supported lookup paths.
    pub parent: Vec<u8>,
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub format: ListFormat,
    /// The constructor's 1.
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

/// A `positionType`'s fields, read by name.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionType {
    pub position: [i32; 2],
    /// The constructor's 1.0.
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

/// A `listBoxType`'s or `smoothListboxType`'s fields and defaults. Every integer point is
/// written straight into the field.
#[derive(Clone, Debug, PartialEq)]
pub struct ListBox {
    /// `offset`.
    pub offset: [i32; 2],
    /// `size`.
    pub size: [i32; 2],
    /// `position`.
    pub position: [i32; 2],
    /// `borderSize`.
    pub border_size: [i32; 2],
    /// `parent`: list box post-load pass never finishes for one that is set (the element is
    /// refused).
    pub parent: Vec<u8>,
    /// `scrollbarType`.
    pub scrollbar_type: Vec<u8>,
    /// `priority` (`%f`), which the constructor leaves uninitialised: `None` until assigned.
    pub priority: Option<f32>,
    /// `horizontal` (`%i`, 0).
    pub horizontal: i32,
    /// `background`: a name, not a block.
    pub background: Vec<u8>,
    /// `dropDown`.
    pub drop_down: Vec<u8>,
    /// `heading`.
    pub heading: Vec<u8>,
    /// `step` and `spacing` (`%i`, 1).
    pub spacing: i32,
    /// `orientation`, kept as its text.
    pub orientation: Vec<u8>,
    /// `membername`.
    pub member_name: Vec<u8>,
    /// `alwaystransparent`/`allwaystransparent`.
    pub always_transparent: bool,
    /// `clipping` (1).
    pub clipping: bool,
    /// `scrollbar_side`: `left` 0, `right` 1 under ASCII case-insensitive comparison; anything else
    /// is logged and kept (1).
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

/// An `editBoxType`'s fields and defaults. `position`, `size` and `borderSize` use the integer
/// component reader, which writes consecutive `i32`s without a count bound. The twelve 32-bit slots
/// from `position` to the object's end are kept as the game holds them ([`EditBox::slots`]);
/// accessors read them in the constructor's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct EditBox {
    /// `font`, initially the null font.
    pub font: Vec<u8>,
    /// `textureFile`.
    pub texture: Vec<u8>,
    /// `text`.
    pub text: Vec<u8>,
    /// From `position` to the object's end: position, size, borderSize, cursor, orientation, the
    /// flag byte and `is_multiline`'s byte (each in its own 32-bit slot), and the object's last
    /// padding.
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

    /// `cursor`, integer point reader into a zeroed local, both halves copied.
    pub fn cursor(&self) -> [i32; 2] {
        [self.slots[6], self.slots[7]]
    }

    /// `orientation`: text orientation reader of each assignment's text, as stored
    /// ([`Orientation::from_gui_value`]).
    pub fn orientation(&self) -> i32 {
        self.slots[8]
    }

    /// The flag byte (6): `ignore_tab_navigation` 0x1, `instantTextBoxType` 0x2,
    /// `use_special_chars` 0x4, `only_numbers` 0x8, `ignore_enter` 0x10.
    pub fn flags(&self) -> u8 {
        self.slots[9].to_le_bytes()[0]
    }

    /// `is_multiline` (1).
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

/// A `scrollbarType`'s fields and defaults. Its children (`guiButtonType`, `buttonType`,
/// `iconType`) belong to the element, with later names replacing earlier ones in its map. The name
/// fields refer to those children.
#[derive(Clone, Debug, PartialEq)]
pub struct Scrollbar {
    /// `position`, `size`, `borderSize`: integer point reader into the field.
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub border_size: [i32; 2],
    /// The eight words that `slider_pos_shift` (integer component reader) writes from its start
    /// with no bound: `slider_pos_shift` x and y, then the bits of `maxValue` (0.0), `minValue`
    /// (uninitialised), `stepSize`, `startValue`, `priority` and padding.
    pub slots: [u32; 8],
    /// Whether `minValue` has been written (the constructor leaves it uninitialised).
    pub min_value_set: bool,
    /// `horizontal` (`%i`, 0).
    pub horizontal: i32,
    /// `scroll_speed` (`%i`, 1).
    pub scroll_speed: i32,
    /// `lockable`, `drag` (1), `useRangeLimit`.
    pub lockable: bool,
    pub drag: bool,
    pub use_range_limit: bool,
    /// `rangeLimitMin` and `rangeLimitMax`, fixed5 reader: the value times 100000.
    pub range_limit_min: i64,
    pub range_limit_max: i64,
    /// `slider`, `track`, `rightButton`, `leftButton`, `rangeLimitMinIcon`, `rangeLimitMaxIcon`:
    /// child names.
    pub slider: Vec<u8>,
    pub track: Vec<u8>,
    pub right_button: Vec<u8>,
    pub left_button: Vec<u8>,
    pub range_limit_min_icon: Vec<u8>,
    pub range_limit_max_icon: Vec<u8>,
    /// `parent`: scrollbar post-load pass copies unset fields from it.
    pub parent: Vec<u8>,
    /// The count of the listed field ids inspected by the unset-field test and The post-load pass.
    /// The constructor pushes twelve ([`SCROLLBAR_LISTED`]) into a block of thirteen; a
    /// `slider_pos_shift` component can write the count.
    pub listed: i32,
}

/// The field ids the constructor initially pushes into the listed-field vector, in order: track,
/// slider, rightButton, leftButton, position, priority, size, borderSize, maxValue, minValue,
/// stepSize, startValue.
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

    /// `None` while uninitialised.
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

/// An `extendedScrollbarType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtendedScrollbar {
    pub position: Position,
    pub size: Size,
    /// `tileSize` (size reader).
    pub tile_size: Size,
    /// `orientation` and `origo`, keyword orientation reader.
    pub orientation: Orientation,
    pub origo: Orientation,
    /// The fixed5 reader (the value times 100000): `maxValue`, `minValue`, `stepSize` (100000),
    /// `startValue` and `smooth_scrolling` (100000; a number here, not a flag).
    pub max_value: i64,
    pub min_value: i64,
    pub step_size: i64,
    pub start_value: i64,
    pub smooth_scrolling: i64,
    /// `horizontal`, `lockable`, `drag` (1), `clickonly`, `setTrackFrameOnChange`.
    pub horizontal: bool,
    pub lockable: bool,
    pub drag: bool,
    pub click_only: bool,
    pub set_track_frame_on_change: bool,
    /// The five button parts, each a button read once: `background`, `track`, `slider`,
    /// `increaseButton`, `decreaseButton`.
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

/// A `dropDownBoxType`'s fields and defaults. Every key but its own and `name` goes to the inner
/// `containerWindowType`: that container's keys, children and common element fields (`hide`,
/// tooltips) belong to the inner element.
#[derive(Clone, Debug, PartialEq)]
pub struct DropDown {
    /// `position`.
    pub position: Position,
    /// `clipping` (1), `expandedOnTop` (1), `hideHeaderOnExpand`, `contractOnLeave`.
    pub clipping: bool,
    pub expanded_on_top: bool,
    pub hide_header_on_expand: bool,
    pub contract_on_leave: bool,
    /// `switch_frame_on_expand`, which the constructor leaves uninitialised: `None` until assigned.
    pub switch_frame_on_expand: Option<bool>,
    /// `expandButton` (a button) and `expandedWindow` (a container), each read once.
    pub expand_button: Option<Box<Element>>,
    pub expanded_window: Option<Box<Element>>,
    /// The inner container, never read as an object: no name, no counter.
    pub inner: Box<Element>,
}

/// A `browserType`'s or `curveGraphType`'s fields: `position` and `size` use the integer
/// component reader into four consecutive `i32`s (position's third and fourth components land
/// in size), followed by `spriteType`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browser {
    /// position x, y, then size x, y.
    pub slots: [i32; 4],
    /// `spriteType`.
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

/// One `windowType` axis of `size` (window size reader): a number, or `max_resolution`, the GUI
/// root's extent less the show position (or the position when that is 0,0) as read so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowExtent {
    Pixels(i32),
    MaxResolution { less: i32 },
}

/// A `windowType`'s fields and constructor defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// `position`, `show_position`, `hide_position`: The integer point reader.
    pub position: [i32; 2],
    pub show_position: [i32; 2],
    pub hide_position: [i32; 2],
    /// `size`.
    pub size: [WindowExtent; 2],
    /// `orientation`: text orientation reader of the lowered text.
    pub orientation: Orientation,
    /// `animation_type`: `accelerated` 0, `decelerated` 1, `smoothstep` 2, `linear` 3, anything
    /// else 1 (1).
    pub animation_type: i32,
    /// `animation_time` (10).
    pub animation_time: i32,
    /// `moveable` (`%i`, 1).
    pub moveable: i32,
    /// `click_to_front`, `fullScreen`.
    pub click_to_front: bool,
    pub full_screen: bool,
    /// `priority`, which the constructor leaves uninitialised: `None` until assigned.
    pub priority: Option<f32>,
    /// `parent`, `background` (a name), `verticalScrollbar`, `horizontalScrollbar`,
    /// `horizontalBorder`, `verticalBorder`, `upsound`, `downsound`.
    pub parent: Vec<u8>,
    pub background: Vec<u8>,
    pub vertical_scrollbar: Vec<u8>,
    pub horizontal_scrollbar: Vec<u8>,
    pub horizontal_border: Vec<u8>,
    pub vertical_border: Vec<u8>,
    pub up_sound: Vec<u8>,
    pub down_sound: Vec<u8>,
    /// The indices in [`Element::children`] of the `guiButtonType` and `multiSpriteButtonType`
    /// children read while `background` already named them. Each is in the name tree but in no
    /// kind's list or order list, so construction builds it only as the background.
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

/// A `textBoxType`'s fields and constructor defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSprite {
    /// `font`, `textureFile`, `text`.
    pub font: Vec<u8>,
    pub texture: Vec<u8>,
    pub text: Vec<u8>,
    /// `position` and `borderSize`: integer point reader.
    pub position: [i32; 2],
    pub border_size: [i32; 2],
    /// `format`: `left` 0, `right` 1 and `centre` 2 by keyword; `center` and anything else leave it
    /// unchanged.
    pub format: i32,
    /// `maxWidth` and `maxHeight`: `%i`.
    pub max_width: i32,
    pub max_height: i32,
    /// `fixedsize`.
    pub fixed_size: bool,
    /// `orientation`: text box post-load pass parses the last text.
    pub orientation: Orientation,
    /// `textblock`: each text block read without an error, in order.
    pub text_blocks: Vec<TextBlock>,
}

/// An element's fields, by kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Fields {
    Container(Box<Container>),
    /// An `iconType`.
    Icon(Icon),
    /// A `buttonType`, `checkboxType`, `guiButtonType`, `multiSpriteButtonType` or `background`.
    Button(Button),
    TextBox(TextBox),
    Grid(Grid),
    List(List),
    PositionType(PositionType),
    /// A `listBoxType` or `smoothListboxType`.
    ListBox(ListBox),
    EditBox(EditBox),
    Scrollbar(Scrollbar),
    ExtendedScrollbar(ExtendedScrollbar),
    DropDown(DropDown),
    /// A `browserType` or `curveGraphType`.
    Browser(Browser),
    Window(Window),
    TextSprite(TextSprite),
    /// A kind no container or window builds, refused.
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

/// One element the game builds, as it builds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: Kind,
    /// The kind keyword as authored.
    pub spelling: String,
    /// The full path: the template's name, then each child's.
    pub path: String,
    name: Vec<u8>,
    /// `hide`: copied to the instance's hidden flag (bit 4 of its state byte), which prevents the
    /// show operation from showing it.
    pub hide: bool,
    /// The other element fields.
    pub common: Common,
    pub fields: Fields,
    /// The direct children the element keeps, in declaration order.
    pub children: Vec<Element>,
    refusals: Vec<Refusal>,
    node: usize,
}

impl Element {
    /// A drop-down's inner container before it is read.
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

    /// The native name, lossy for display: the last `name`, or the name the game generated.
    pub fn name(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.name)
    }

    /// The native name's bytes, as the game's name trees compare them.
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Whether the name is `name` under the game's name-tree comparison: each byte lowered.
    pub fn is_named(&self, name: impl AsRef<[u8]>) -> bool {
        self.name.eq_ignore_ascii_case(name.as_ref())
    }

    /// The refusals of the element's own keys, and of any walk the game itself does not define
    /// before the element ended.
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }

    /// The elements this one holds: its backgrounds and parts, then its children in declaration
    /// order.
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

    /// The element's refusals and every kept descendant's (backgrounds and parts included), in
    /// document order.
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

    /// Window-local lookup of one kind: the child of that kind named `name`, or `None` for the
    /// lookup's null control. `windowType` and `scrollbarType` keep the last child of a name.
    /// Position types use [`Self::find_instance`]'s exact name check.
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

    /// This window's position-type name array first compares length and bytes, then queries its
    /// name tree. No case folding or recursion.
    pub fn find_instance(&self, name: impl AsRef<[u8]>) -> Option<&Element> {
        let name = name.as_ref();
        self.children
            .iter()
            .find(|child| child.kind == Kind::PositionType && child.name == name)
    }

    /// Recursive kind lookup through child containers in attach order. The instant-text lookup uses
    /// its own array when setting text.
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

    /// This window's own list of that name, else the first found below each child container in
    /// declaration order.
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

    /// The child of any kind named `name` as the element's name tree finds it.
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

    /// Follow the kept nested controls by name, including container backgrounds.
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

    /// A `windowType`'s fields, when the element is one.
    pub fn window(&self) -> Option<&Window> {
        match &self.fields {
            Fields::Window(window) => Some(window),
            _ => None,
        }
    }

    /// A button's fields.
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

/// Dropped with a stack of its own, so a tree nested as deep as a file can nest it is freed
/// without recursing.
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

/// One file as the game reads it: its templates in registration order, which of them the
/// file's own `guiTypes` blocks leave registered under each name, and what its level refuses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gui {
    /// Every element `guiTypes` registers, in the order the blocks read them.
    templates: Vec<Element>,
    /// The `default_*sound` keys the file sets.
    default_sounds: DefaultSounds,
    /// Each lowered name, and the template the file's inserts leave under it.
    registered: BTreeMap<Vec<u8>, usize>,
    refusals: Vec<Refusal>,
}

impl Gui {
    /// Template lookup over this file alone: the template its inserts leave under `name`,
    /// compared with every byte lowered.
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

    /// Every template the file read, in the order its blocks read them.
    pub fn templates(&self) -> &[Element] {
        &self.templates
    }

    /// Whether a `textBoxType` the file reads is named `name`, byte for byte. Templates and
    /// `windowType` children are the places that keep one. Construction registers a new text
    /// sprite under that name for the rest of the session when no sprite has it.
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

    /// What the file's own level and its `guiTypes` blocks hold that the game does not define.
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
}

/// Reader state shared by every `.gui` file: the counter used to name unnamed elements,
/// initially 0 and never reset.
#[derive(Clone, Debug, Default)]
pub struct Context {
    unnamed: i32,
}

/// Reads one `.gui` file as [`read`] does, the GUI root's extent at load being `viewport`: the
/// reading client's own (the viewer's GUI viewport, a dump's resolution), which a `windowType`'s
/// `max_resolution` size holds less its number when the post-load pass tests it.
pub fn read_at(bytes: &[u8], viewport: [i32; 2]) -> Gui {
    let mut gui = Context::new().read(bytes);
    inherit::pass(&mut [&mut gui], Some(viewport));
    gui
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read one file after all files already read by this context, in the game's load order.
    /// Its post-load pass is left to the caller (see [`read_layers`]).
    pub fn read(&mut self, bytes: &[u8]) -> Gui {
        let mut reader = reader::Reader::new(
            crate::parser::native::lex(bytes, crate::parser::native::Mode::KeepMark),
            self.unnamed,
        );
        let blocks = reader.file();
        self.unnamed = reader.unnamed;
        finish(reader, blocks)
    }

    /// Reads `files` in order as [`Self::read`] reads each in turn, parsing them on the
    /// machine's threads. A file's generated names continue the counter where the files before
    /// it left it, and how far a file moves the counter does not depend on where it starts: so
    /// every file is read from 0 at once, and a file that names elements is read again from its
    /// place in the count, unless that place is 0.
    pub fn read_all<B: AsRef<[u8]> + Sync>(&mut self, files: &[B]) -> Vec<Gui> {
        let read_from = |bytes: &[u8], base: i32| {
            let mut context = Context { unnamed: base };
            let gui = context.read(bytes);
            (gui, context.unnamed.wrapping_sub(base))
        };
        let first = crate::parallel::map(files, |bytes| read_from(bytes.as_ref(), 0));
        let mut again = Vec::new();
        let mut guis = Vec::with_capacity(first.len());
        for (index, (gui, named)) in first.into_iter().enumerate() {
            if named != 0 && self.unnamed != 0 {
                again.push((index, self.unnamed));
            }
            self.unnamed = self.unnamed.wrapping_add(named);
            guis.push(gui);
        }
        let reread = crate::parallel::map(&again, |&(index, base)| {
            read_from(files[index].as_ref(), base).0
        });
        for ((index, _), gui) in again.into_iter().zip(reread) {
            guis[index] = gui;
        }
        guis
    }
}

/// Reads one `.gui` file as the first the game reads (the name counter at 0), with the post-load
/// pass over its own templates, as if it were the only file.
pub fn read(bytes: &[u8]) -> Gui {
    let mut gui = Context::new().read(bytes);
    inherit::pass(&mut [&mut gui], None);
    gui
}

/// The first file read, before enumerating `interface`.
pub const LOAD_SCREEN: &str = "interface/load_screen.gui";

/// Every `.gui` file of the content stack in the game's load order with one [`Context`]:
/// `interface/load_screen.gui` first, followed by its post-load pass; then `interface/**/*.gui`
/// excluding that file, each directory's files in byte order before its subdirectories, depth
/// first. Finally run the post-load pass over every template.
pub fn read_layers(layers: &ResolvedLayers) -> io::Result<Guis> {
    read_layers_with(layers, None)
}

/// [`read_layers`] with the GUI root's extent at load supplied as `viewport`. This is the reading
/// client's extent: device size clamped to `min_gui`, divided by UI scale and truncated (the
/// viewer's GUI viewport or a dump's resolution). A `windowType`'s `max_resolution` axis stores
/// this extent less its number when the post-load pass tests whether it is set. Without the extent,
/// that window is refused ([`Cause::Unrecorded`]).
pub fn read_layers_at(layers: &ResolvedLayers, viewport: [i32; 2]) -> io::Result<Guis> {
    read_layers_with(layers, Some(viewport))
}

fn read_layers_with(layers: &ResolvedLayers, viewport: Option<[i32; 2]>) -> io::Result<Guis> {
    let definitions = layers.product("GUI definitions before inheritance", 1, &[], read_layer_definitions)?;
    let mut files = definitions.as_ref().clone();
    if let Some((path, gui)) = files.first_mut()
        && path == LOAD_SCREEN
    {
        inherit::pass(&mut [gui], viewport);
    }
    inherit::pass(
        &mut files.iter_mut().map(|(_, gui)| gui).collect::<Vec<_>>(),
        viewport,
    );
    Ok(Guis::new(files))
}

fn read_layer_definitions(layers: &ResolvedLayers) -> io::Result<Vec<(String, Gui)>> {
    let mut context = Context::new();
    let mut files: Vec<(String, Gui)> = Vec::new();
    // The layers' own bytes, as the game's enumeration returns them; each file is read by them
    // and labelled with their text.
    let listed = layers.listed_layer_paths(b"interface/")?;
    let label = |path: &[u8]| String::from_utf8_lossy(path).into_owned();
    // Open the literal lowercase path through the virtual file system: folder lookup accepts
    // any case, archive lookup uses exact bytes. A missing file is passed.
    if let Some(bytes) = crate::content::open_request(layers, LOAD_SCREEN.as_bytes())? {
        files.push((LOAD_SCREEN.to_owned(), context.read(&bytes)));

    }
    // The main pass drops exactly this name.
    let paths: Vec<Vec<u8>> =
        crate::defines::load_order_paths(b"interface/", b".gui", listed.iter().map(Vec::as_slice))
            .into_iter()
            .filter(|path| path != LOAD_SCREEN.as_bytes())
            .collect();
    // Read and parsed on the machine's threads, each file exactly as it would be read alone in
    // its turn; the first that fails to read fails the whole, as in order.
    let texts = layers
        .read_paths(&paths)
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
    for (path, gui) in paths.iter().zip(context.read_all(&texts)) {
        files.push((label(path), gui));
    }
    Ok(files)
}

/// Every `.gui` file the game reads, in its load order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guis {
    pub files: Vec<(String, Gui)>,
    /// Lowered names in byte order and the file/template each last registration selects.
    registered: Vec<(Vec<u8>, usize, usize)>,
}

impl Guis {
    /// The default button sounds in force after every file ([`DefaultSounds`]).
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

    /// Builds the shared template registry in file registration order.
    pub fn new(files: Vec<(String, Gui)>) -> Self {
        let mut registered = BTreeMap::new();
        for (file, (_, gui)) in files.iter().enumerate() {
            for (name, &template) in &gui.registered {
                registered.insert(name.clone(), (file, template));
            }
        }
        Self {
            files,
            registered: registered
                .into_iter()
                .map(|(name, (file, template))| (name, file, template))
                .collect(),
        }
    }

    /// Template lookup over the shared registry: the template left by the last file registering
    /// `name`.
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

    /// [`Gui::text_box_named`] over every file.
    pub fn text_box_named(&self, name: &[u8]) -> bool {
        self.files.iter().any(|(_, gui)| gui.text_box_named(name))
    }

    /// Every file's own refusals, with its path.
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

/// Insertion order of a `guiTypes` block's pending templates after stable bytewise name
/// sorting: visit each index in order, replacing an earlier equal name with the later insert.
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

/// Resolve each refusal's path once every element's name is known, and register every
/// `guiTypes` block's templates in the game's insertion order.
fn finish(reader: reader::Reader, blocks: Vec<Vec<Element>>) -> Gui {
    let mut paths: Vec<String> = Vec::with_capacity(reader.labels.len());
    for label in &reader.labels {
        // A label's parent is always made before it.
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

/// The GUI's default button sounds: each key's last setting in load order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultSounds {
    pub click: Option<Vec<u8>>,
    pub over: Option<Vec<u8>>,
    pub down: Option<Vec<u8>>,
    pub up: Option<Vec<u8>>,
}

/// The sound effects a button plays entering Over, Down and Up, and on its click.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonSounds {
    pub over: Option<Vec<u8>>,
    pub down: Option<Vec<u8>>,
    pub up: Option<Vec<u8>>,
    pub click: Option<Vec<u8>>,
    /// A check box's on unchecking: its own `upsound`, else its click.
    pub uncheck: Option<Vec<u8>>,
}

impl Button {
    /// As the game resolves them when it builds the button; `no_clicksound` withholds defaults.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
