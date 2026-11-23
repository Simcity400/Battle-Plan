use content_reader_script::localization_binding::{BoundArgument, BoundLocalization, BoundNode, BoundValue, FormattedLocalization};

use content_reader_script::native::{Kind as TokenKind, Token};
use content_reader_parser::parser::read_bool;

use super::{
    Animation, Browser, Button, Cause,
    Common, Container, DropDown, EditBox, Element, ExtendedScrollbar, Fields,
    Grid, GridFormat, Icon, Kind, List, ListBox, ListFormat, Orientation,
    Position, PositionType, Refusal, Scrollbar, Size, TextBlock, TextBox, TextFormat, TextSprite,
    VerticalAlignment, Window, WindowExtent,
    reader::{Axis, Reader, Refused, fnv64, keyword_of, read_string, record},
    spill::{self, Layout},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Build {
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
    Opaque(&'static str),
}

impl Build {
    fn kind(self) -> Kind {
        match self {
            Self::Container => Kind::Container,
            Self::Icon => Kind::Icon,
            Self::Button => Kind::Button,
            Self::Checkbox => Kind::Checkbox,
            Self::GuiButton => Kind::GuiButton,
            Self::MultiSpriteButton => Kind::MultiSpriteButton,
            Self::TextBox => Kind::TextBox,
            Self::Grid => Kind::Grid,
            Self::List => Kind::List,
            Self::PositionType => Kind::PositionType,
            Self::ListBox => Kind::ListBox,
            Self::SmoothListBox => Kind::SmoothListBox,
            Self::TextSprite => Kind::TextSprite,
            Self::EditBox => Kind::EditBox,
            Self::Scrollbar => Kind::Scrollbar,
            Self::ExtendedScrollbar => Kind::ExtendedScrollbar,
            Self::DropDown => Kind::DropDown,
            Self::Browser => Kind::Browser,
            Self::CurveGraph => Kind::CurveGraph,
            Self::Window => Kind::Window,
            Self::Opaque(keyword) => Kind::Other(keyword),
        }
    }

    fn fields(self) -> Fields {
        match self {
            Self::Container => Fields::Container(Box::default()),
            Self::Icon => Fields::Icon(Icon::default()),
            Self::Button | Self::Checkbox | Self::GuiButton | Self::MultiSpriteButton => {
                Fields::Button(Button::default())
            }
            Self::TextBox => Fields::TextBox(TextBox::default()),
            Self::Grid => Fields::Grid(Grid::default()),
            Self::List => Fields::List(List::default()),
            Self::PositionType => Fields::PositionType(PositionType::default()),
            Self::ListBox | Self::SmoothListBox => Fields::ListBox(ListBox::default()),
            Self::TextSprite => Fields::TextSprite(TextSprite::default()),
            Self::EditBox => Fields::EditBox(EditBox::default()),
            Self::Scrollbar => Fields::Scrollbar(Scrollbar::default()),
            Self::ExtendedScrollbar => Fields::ExtendedScrollbar(ExtendedScrollbar::default()),
            Self::DropDown => Fields::DropDown(DropDown {
                position: Position::default(),
                clipping: true,
                expanded_on_top: true,
                hide_header_on_expand: false,
                contract_on_leave: false,
                switch_frame_on_expand: None,
                expand_button: None,
                expanded_window: None,
                inner: Box::new(Element::placeholder()),
            }),
            Self::Browser | Self::CurveGraph => Fields::Browser(Browser::default()),
            Self::Window => Fields::Window(Window::default()),
            Self::Opaque(_) => Fields::Other,
        }
    }
}

fn container_child(key: &str) -> Option<Build> {
    Some(match key {
        "containerwindowtype" => Build::Container,
        "icontype" => Build::Icon,
        "buttontype" => Build::Button,
        "checkboxtype" => Build::Checkbox,
        "instanttextboxtype" => Build::TextBox,
        "gridboxtype" => Build::Grid,
        "overlappingelementsboxtype" => Build::List,
        "positiontype" => Build::PositionType,
        "listboxtype" => Build::ListBox,
        "smoothlistboxtype" => Build::SmoothListBox,
        "editboxtype" => Build::EditBox,
        "extendedscrollbartype" => Build::ExtendedScrollbar,
        "dropdownboxtype" => Build::DropDown,
        "browsertype" => Build::Browser,
        "curvegraphtype" => Build::CurveGraph,
        "scrollbartype" => Build::Scrollbar,
        _ => return None,
    })
}

fn window_child(key: &str) -> Option<Build> {
    Some(match key {
        "icontype" => Build::Icon,
        "checkboxtype" => Build::Checkbox,
        "multispritebuttontype" => Build::MultiSpriteButton,
        "guibuttontype" => Build::GuiButton,
        "instanttextboxtype" => Build::TextBox,
        "textboxtype" => Build::TextSprite,
        "editboxtype" => Build::EditBox,
        "windowtype" => Build::Window,
        "listboxtype" => Build::ListBox,
        "smoothlistboxtype" => Build::SmoothListBox,
        "overlappingelementsboxtype" => Build::List,
        "scrollbartype" => Build::Scrollbar,
        "browsertype" => Build::Browser,
        "curvegraphtype" => Build::CurveGraph,
        "dropdownboxtype" => Build::DropDown,
        _ => return None,
    })
}

fn template_kind(key: &str) -> Option<Build> {
    Some(match key {
        "containerwindowtype" => Build::Container,
        "guibuttontype" => Build::GuiButton,
        "textboxtype" => Build::TextSprite,
        "scrollbartype" => Build::Scrollbar,
        "windowtype" => Build::Window,
        "listboxtype" => Build::ListBox,
        "editboxtype" => Build::EditBox,
        "positiontype" => Build::PositionType,
        "icontype" => Build::Icon,
        "instanttextboxtype" => Build::TextBox,
        "overlappingelementsboxtype" => Build::List,
        "extendedscrollbartype" => Build::ExtendedScrollbar,
        "dropdownboxtype" => Build::DropDown,
        "browsertype" => Build::Browser,
        "curvegraphtype" => Build::CurveGraph,
        "radiobuttongrouptype" => Build::Opaque("radioButtonGroupType"),
        "scrollbargrouptype" => Build::Opaque("scrollbargroupType"),
        "layout" => Build::Opaque("layout"),
        "dropdownmenutype" => Build::Opaque("dropDownMenuType"),
        _ => return None,
    })
}

const DEFAULT_SOUNDS: [&str; 4] = [
    "default_clicksound",
    "default_oversound",
    "default_downsound",
    "default_upsound",
];

const DROP_DOWN_REFUSED: [&str; 12] = [
    "show_position",
    "hide_position",
    "animation_type",
    "animation_time",
    "show_animation_type",
    "hide_animation_type",
    "show_animation_time",
    "hide_animation_time",
    "autohide_scrollbars",
    "drag_scroll",
    "verticalscrollbar",
    "horizontalscrollbar",
];

pub(super) struct Building {
    build: Build,
    spelled: String,
    node: usize,
    name: Vec<u8>,
    hide: bool,
    common: Common,
    fields: Fields,
    children: Vec<Element>,
    refusals: Vec<Refusal>,
    refused: Refused,
    orientation_text: Option<Vec<u8>>,
    grid_background: bool,
    named: bool,
    inner: Option<Box<Building>>,
    sprite_string: spill::CxxString,
    factory: Option<String>,
}

impl Building {
    fn store<T>(&mut self, value: Option<T>, slot: impl FnOnce(&mut Fields, T)) {
        if let Some(value) = value {
            slot(&mut self.fields, value);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    ScrollbarBackground,
    Track,
    Slider,
    IncreaseButton,
    DecreaseButton,
    ExpandButton,
    ExpandedWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attach {
    Root,
    Child,
    Background,
    GridBackground,
    InnerChild,
    InnerBackground,
    Part(Part),
}

struct Nested {
    build: Build,
    spelled: String,
    attach: Attach,
}

enum Handled {
    Yes,
    Opens(Nested),
    No,
}

impl Handled {
    fn child(build: Build, key: &Token) -> Self {
        Self::Opens(Nested {
            build,
            spelled: spelling(&key.text),
            attach: Attach::Child,
        })
    }

    fn part(build: Build, key: &Token, attach: Attach) -> Self {
        Self::Opens(Nested {
            build,
            spelled: spelling(&key.text),
            attach,
        })
    }
}

impl Reader<'_> {
    pub(super) fn file(&mut self) -> Vec<Vec<Element>> {
        let mut blocks = Vec::new();
        while self.next_key() {
            let key = self.key.clone();
            match keyword_of(&key) {
                Some("guitypes") => {
                    let block = self.gui_types();
                    blocks.push(block);
                }
                Some(word) if DEFAULT_SOUNDS.contains(&word) => {
                    let value = Some(read_string(&self.value.text));
                    let sounds = &mut self.default_sounds;
                    match word {
                        "default_clicksound" => sounds.click = value,
                        "default_oversound" => sounds.over = value,
                        "default_downsound" => sounds.down = value,
                        _ => sounds.up = value,
                    }
                }
                _ => self.unexpected(),
            }
        }
        blocks
    }

    fn gui_types(&mut self) -> Vec<Element> {
        let mut templates = Vec::new();
        while self.next_key() {
            let key = self.key.clone();
            match keyword_of(&key).and_then(template_kind) {
                Some(build) => {
                    let element = self.element(build, &spelling(&key.text), None);
                    templates.push(element);
                }
                None => self.unexpected(),
            }
        }
        templates
    }

    fn open(
        &mut self,
        build: Build,
        spelled: &str,
        parent: Option<usize>,
        named: bool,
    ) -> Building {
        let node = self.label(parent, "");
        let mut element = Building {
            build,
            spelled: spelled.to_owned(),
            node,
            name: Vec::new(),
            hide: false,
            common: Common::default(),
            fields: build.fields(),
            children: Vec::new(),
            refusals: Vec::new(),
            refused: Refused::default(),
            orientation_text: None,
            grid_background: false,
            named,
            inner: None,
            sprite_string: spill::CxxString::default(),
            factory: None,
        };
        match build {
            Build::DropDown => {
                let inner = self.open(Build::Container, "", Some(node), false);
                element.inner = Some(Box::new(inner));
            }
            Build::Opaque(keyword) => record(
                &mut element.refusals,
                node,
                "",
                format!(
                    "is a {keyword}, a guiTypes kind no container or window builds, which this \
                     reader does not read (its block is read to its end)"
                ),
                Cause::Unread,
            ),
            _ => {}
        }
        element
    }

    fn element(&mut self, build: Build, spelled: &str, parent: Option<usize>) -> Element {
        let mut stack = vec![(self.open(build, spelled, parent, true), Attach::Root)];
        loop {
            let (top, _) = stack.last_mut().expect("the driver's element is open");
            if let Build::Opaque(_) = top.build {
                self.skip_reported();
            } else if self.next_key() {
                let key = self.key.clone();
                if let Some(nested) = self.dispatch(top, &key) {
                    let label = match nested.attach {
                        Attach::InnerChild | Attach::InnerBackground => {
                            top.inner.as_ref().map_or(top.node, |inner| inner.node)
                        }
                        _ => top.node,
                    };
                    let named = !matches!(
                        nested.attach,
                        Attach::Background
                            | Attach::GridBackground
                            | Attach::InnerBackground
                            | Attach::Part(Part::ScrollbarBackground)
                    );
                    let opened = self.open(nested.build, &nested.spelled, Some(label), named);
                    stack.push((opened, nested.attach));
                }
                continue;
            }
            let (done, attach) = stack.pop().expect("the driver's element is open");
            let element = self.finish_element(done);
            match stack.last_mut() {
                Some((parent, _)) => attach_to(parent, attach, element),
                None => return element,
            }
        }
    }

    fn dispatch(&mut self, element: &mut Building, key: &Token) -> Option<Nested> {
        let Some(word) = keyword_of(key) else {
            self.unexpected();
            return None;
        };
        if element.build == Build::DropDown {
            return self.drop_down_key(element, word, key);
        }
        let own = match element.build {
            Build::Container => self.container_key(element, word),
            Build::Icon => self.icon_key(element, word),
            Build::Button | Build::Checkbox | Build::GuiButton | Build::MultiSpriteButton => {
                self.button_key(element, word)
            }
            Build::TextBox => self.text_box_key(element, word),
            Build::Grid => self.grid_key(element, word),
            Build::List => self.list_key(element, word),
            Build::PositionType => self.position_type_key(element, word),
            Build::ListBox | Build::SmoothListBox => self.list_box_key(element, word),
            Build::TextSprite => self.text_sprite_key(element, word),
            Build::EditBox => self.edit_box_key(element, word),
            Build::Scrollbar => self.scrollbar_key(element, word),
            Build::ExtendedScrollbar => self.extended_scrollbar_key(element, word),
            Build::Browser | Build::CurveGraph => self.browser_key(element, word),
            Build::Window => self.window_key(element, word),
            Build::DropDown | Build::Opaque(_) => Handled::No,
        };
        match own {
            Handled::Yes => None,
            Handled::Opens(nested) => Some(nested),
            Handled::No => {
                if !self.generic_key(element, word) {
                    self.unexpected();
                }
                None
            }
        }
    }

    fn drop_down_key(&mut self, element: &mut Building, word: &str, key: &Token) -> Option<Nested> {
        let text = self.value.text.clone();
        let Fields::DropDown(fields) = &mut element.fields else {
            return None;
        };
        match word {
            "name" => element.name = read_string(&text),
            "position" => {
                let mut position = fields.position;
                self.position(&mut element.refused, "position", &mut position);
                if let Fields::DropDown(fields) = &mut element.fields {
                    fields.position = position;
                }
            }
            "clipping"
            | "expandedontop"
            | "hideheaderonexpand"
            | "contractonleave"
            | "switch_frame_on_expand" => {
                if let Some(flag) = read_bool(&text) {
                    match word {
                        "clipping" => fields.clipping = flag,
                        "expandedontop" => fields.expanded_on_top = flag,
                        "hideheaderonexpand" => fields.hide_header_on_expand = flag,
                        "contractonleave" => fields.contract_on_leave = flag,
                        _ => fields.switch_frame_on_expand = Some(flag),
                    }
                }
            }
            "expandbutton" => {
                if fields.expand_button.is_none() {
                    return Some(Nested {
                        build: Build::Button,
                        spelled: spelling(&key.text),
                        attach: Attach::Part(Part::ExpandButton),
                    });
                }
            }
            "expandedwindow" => {
                if fields.expanded_window.is_none() {
                    return Some(Nested {
                        build: Build::Container,
                        spelled: spelling(&key.text),
                        attach: Attach::Part(Part::ExpandedWindow),
                    });
                }
            }
            _ if DROP_DOWN_REFUSED.contains(&word) => self.unexpected(),
            _ => {
                let mut inner = element.inner.take().expect("a drop-down reads its inner");
                let own = self.container_key(&mut inner, word);
                let nested = match own {
                    Handled::Yes => None,
                    Handled::Opens(mut nested) => {
                        nested.attach = match nested.attach {
                            Attach::Background => Attach::InnerBackground,
                            _ => Attach::InnerChild,
                        };
                        Some(nested)
                    }
                    Handled::No => {
                        if !self.generic_key(&mut inner, word) {
                            self.unexpected();
                        }
                        None
                    }
                };
                element.inner = Some(inner);
                return nested;
            }
        }
        None
    }

    fn generic_key(&mut self, element: &mut Building, word: &str) -> bool {
        let text = self.value.text.clone();
        match word {
            "name" => element.name = read_string(&text),
            "dontrender" => {}
            "tooltip" | "tooltiptext" | "pdx_tooltip" => {
                element.common.tooltip = fnv64(&text);
                element.common.tooltip_key = content_reader_parser::parser::c_string(&text).to_vec();
            }
            "delayedtooltiptext" | "pdx_tooltip_delayed" => {
                element.common.delayed_tooltip = fnv64(&text);
                element.common.delayed_tooltip_key = content_reader_parser::parser::c_string(&text).to_vec();
            }
            "pdx_disabled_tooltip" => {
                element.common.disabled_tooltip = fnv64(&text);
                element.common.disabled_tooltip_key = content_reader_parser::parser::c_string(&text).to_vec();
            }
            "pdx_disabled_tooltip_delayed" => {
                element.common.disabled_tooltip_delayed = fnv64(&text);
                element.common.disabled_tooltip_delayed_key =
                    content_reader_parser::parser::c_string(&text).to_vec();
            }
            "hint_tag" => element.common.hint_tag = read_string(&text),
            "hide" => {
                if let Some(hide) = read_bool(&text) {
                    element.hide = hide;
                }
            }
            "pdx_tooltip_anchor" => {
                element.common.tooltip_anchor.set = true;
                self.tooltip_anchor(&mut element.common);
            }
            "context_aware_tooltip" | "bound_tooltip" => {
                if element.common.bound_tooltip.is_empty() {
                    element.common.bound_tooltip = self.bound_localization();
                    if word == "context_aware_tooltip" {
                        element.common.context_aware_tooltip = true;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn tooltip_anchor(&mut self, common: &mut Common) {
        while self.next_key() {
            let key = self.key.clone();
            let text = self.value.text.clone();
            let anchor = &mut common.tooltip_anchor;
            match keyword_of(&key) {
                Some("x") => anchor.x = content_reader_parser::crt::scan_i32(&text).stored().unwrap_or(anchor.x),
                Some("y") => anchor.y = content_reader_parser::crt::scan_i32(&text).stored().unwrap_or(anchor.y),
                Some("relative") => anchor.relative = read_bool(&text).unwrap_or(anchor.relative),
                Some("below") => anchor.below = read_bool(&text).unwrap_or(anchor.below),
                _ => self.unexpected(),
            }
        }
    }

    fn bound_localization(&mut self) -> BoundLocalization {
        let mut bound = BoundLocalization::default();
        if self.value.kind != TokenKind::Open {
            bound.nodes.push(BoundNode {
                key: FormattedLocalization::parse(content_reader_parser::parser::c_string(&self.value.text)),
                arguments: Vec::new(),
            });
            return bound;
        }
        bound.nodes.push(BoundNode::default());
        let mut open = vec![0usize];
        while let Some(&node) = open.last() {
            if self.peek().ends_block() {
                self.read();
                open.pop();
                continue;
            }
            self.key_operator_value();
            let key = self.key.clone();
            let value = self.value.clone();
            if key.is_keyword("localization_key") {
                bound.nodes[node].key =
                    FormattedLocalization::parse(content_reader_parser::parser::c_string(&value.text));
                continue;
            }
            let argument = if value.kind == TokenKind::Open {
                bound.nodes.push(BoundNode::default());
                open.push(bound.nodes.len() - 1);
                BoundValue::Node(bound.nodes.len() - 1)
            } else {
                BoundValue::Localization(FormattedLocalization::parse(content_reader_parser::parser::c_string(
                    &value.text,
                )))
            };
            bound.nodes[node].arguments.push(BoundArgument {
                name: key.c_str().to_vec(),
                value: argument,
            });
        }
        if bound.nodes[0].key.is_empty_key() {
            bound.nodes.clear();
        }
        bound
    }

    fn container_key(&mut self, element: &mut Building, word: &str) -> Handled {
        if let Some(build) = container_child(word) {
            return Handled::child(build, &self.key);
        }
        let text = self.value.text.clone();
        match word {
            "position" => {
                let mut position = container(element).position;
                self.position(&mut element.refused, "position", &mut position);
                let fields = container(element);
                fields.position = position;
                fields.show_position = position;
                fields.hide_position = position;
            }
            "show_position"
            | "hide_position"
            | "closed_position"
            | "open_position"
            | "peek_position"
            | "peek_check_top_left" => {
                let fields = container(element);
                let mut position = match word {
                    "show_position" => fields.show_position,
                    "hide_position" => fields.hide_position,
                    "closed_position" => fields.closed_position,
                    "open_position" => fields.open_position,
                    "peek_position" => fields.peek_position,
                    _ => fields.peek_check_top_left,
                };
                self.position(&mut element.refused, word, &mut position);
                let fields = container(element);
                match word {
                    "show_position" => fields.show_position = position,
                    "hide_position" => fields.hide_position = position,
                    "closed_position" => fields.closed_position = position,
                    "open_position" => fields.open_position = position,
                    "peek_position" => fields.peek_position = position,
                    _ => fields.peek_check_top_left = position,
                }
                if !matches!(word, "show_position" | "hide_position") {
                    fields.peek = true;
                }
            }
            "size" | "peek_check_size" => {
                let fields = container(element);
                let mut size = if word == "size" {
                    fields.size
                } else {
                    fields.peek_check_size
                };
                self.size(&mut element.refused, word, &mut size);
                let fields = container(element);
                if word == "size" {
                    fields.size = size;
                } else {
                    fields.peek_check_size = size;
                    fields.peek = true;
                }
            }
            "orientation" | "origo" => {
                let value = Orientation::from_keyword(&self.value).unwrap_or_default();
                let fields = container(element);
                if word == "orientation" {
                    fields.orientation = value;
                } else {
                    fields.origo = value;
                }
            }
            "clipping" => container(element).clipping = Some(read_bool(&text).unwrap_or(false)),
            "margin" | "background_margin" => {
                let fields = container(element);
                let mut margin = if word == "margin" {
                    fields.margin
                } else {
                    fields.background_margin
                };
                self.margin(&mut element.refused, word, &mut margin);
                let fields = container(element);
                if word == "margin" {
                    fields.margin = margin;
                } else {
                    fields.background_margin = margin;
                }
            }
            "background" => {
                return Handled::Opens(Nested {
                    build: Build::Button,
                    spelled: "background".into(),
                    attach: Attach::Background,
                });
            }
            "scale" | "scroll_wheel_factor" | "vertical_scroll_step" | "horizontal_scroll_step" => {
                element.store(content_reader_parser::crt::scan_f32(&text), |fields, value| {
                    if let Fields::Container(fields) = fields {
                        match word {
                            "scale" => fields.scale = value,
                            "scroll_wheel_factor" => fields.scroll_wheel_factor = value,
                            "vertical_scroll_step" => fields.vertical_scroll_step = value,
                            _ => fields.horizontal_scroll_step = value,
                        }
                    }
                });
            }
            "animation_type" | "show_animation_type" | "hide_animation_type" | "fade_type" => {
                let value = self.animation();
                let fields = container(element);
                match word {
                    "animation_type" => {
                        fields.show_animation = value;
                        fields.hide_animation = value;
                    }
                    "show_animation_type" => fields.show_animation = value,
                    "hide_animation_type" => fields.hide_animation = value,
                    _ => fields.fade = value,
                }
            }
            "fade_time"
            | "fade_delay"
            | "animation_time"
            | "show_animation_time"
            | "hide_animation_time" => {
                element.store(content_reader_parser::crt::scan_i32(&text).stored(), |fields, value| {
                    if let Fields::Container(fields) = fields {
                        match word {
                            "fade_time" => fields.fade_time = value,
                            "fade_delay" => fields.fade_delay = value,
                            "animation_time" => {
                                fields.show_animation_time = value;
                                fields.hide_animation_time = value;
                            }
                            "show_animation_time" => fields.show_animation_time = value,
                            _ => fields.hide_animation_time = value,
                        }
                    }
                });
            }
            "moveable" | "autohide_scrollbars" | "click_to_front" | "smooth_scrolling" => {
                if let Some(flag) = read_bool(&text) {
                    let fields = container(element);
                    match word {
                        "moveable" => fields.moveable = flag,
                        "autohide_scrollbars" => fields.autohide_scrollbars = flag,
                        "click_to_front" => fields.click_to_front = flag,
                        _ => fields.smooth_scrolling = flag,
                    }
                }
            }
            "verticalscrollbar"
            | "horizontalscrollbar"
            | "upsound"
            | "show_sound"
            | "downsound"
            | "hide_sound" => {
                let value = read_string(&text);
                let fields = container(element);
                match word {
                    "verticalscrollbar" => fields.vertical_scrollbar = value,
                    "horizontalscrollbar" => fields.horizontal_scrollbar = value,
                    "upsound" | "show_sound" => fields.show_sound = value,
                    _ => fields.hide_sound = value,
                }
            }
            "drag_scroll" => {
                let value = self.drag_scroll();
                container(element).drag_scroll |= value;
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn animation(&self) -> Animation {
        [
            ("none", Animation::None),
            ("accelerated", Animation::Accelerated),
            ("decelerated", Animation::Decelerated),
            ("smoothstep", Animation::Smoothstep),
            ("linear", Animation::Linear),
        ]
        .iter()
        .find(|(word, _)| self.value.is_keyword(word))
        .map_or(Animation::None, |(_, value)| *value)
    }

    fn drag_scroll(&mut self) -> u8 {
        let button = |token: &Token| -> u8 {
            [("left", 1), ("middle", 2), ("right", 4)]
                .iter()
                .find(|(word, _)| token.is_keyword(word))
                .map_or(0, |(_, value)| *value)
        };
        if self.value.kind != TokenKind::Open {
            return button(&self.value);
        }
        let mut value = 0;
        loop {
            if self.peek().ends_block() {
                self.read();
                return value;
            }
            let token = self.read_resolved();
            if !self.read_ok {
                return value;
            }
            value |= button(&token);
        }
    }

    fn store_point(element: &mut Building, key: &str, point: [Axis; 2], target: &mut [i32; 2]) {
        for (target, axis) in target.iter_mut().zip(point) {
            match axis {
                Axis::Untouched => {}
                Axis::Value(value) => *target = value,
                Axis::Undefined => {
                    element.refused.refuse(
                        key,
                        "scans nothing of its first value, so the integer point reader stores its \
                         uninitialised local: an undefined read"
                            .into(),
                    );
                    return;
                }
            }
        }
    }

    fn icon_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "spritetype" | "quadtexturesprite" | "buttonmesh" => {
                if let Some(icon) = element.fields.icon_mut() {
                    icon.sprite = read_string(&text);
                }
            }
            "position" => {
                let mut point = [0; 2];
                let axes = self.int_point();
                Self::store_point(element, "position", axes, &mut point);
                if let Some(icon) = element.fields.icon_mut() {
                    icon.position = point.map(|value| value as f32);
                }
            }
            "scale" => element.store(content_reader_parser::crt::scan_f32(&text), |fields, value| {
                if let Some(icon) = fields.icon_mut() {
                    icon.scale = value;
                }
            }),
            "orientation" => {
                if let Some(icon) = element.fields.icon_mut() {
                    icon.orientation = Orientation::parse_gui(&read_string(&text));
                }
            }
            "frame" => element.store(content_reader_parser::crt::scan_i32(&text).stored(), |fields, value| {
                if let Some(icon) = fields.icon_mut() {
                    icon.frame = value;
                }
            }),
            "rotation" => element.store(content_reader_parser::crt::scan_f32(&text), |fields, value| {
                if let Some(icon) = fields.icon_mut() {
                    icon.rotation = value;
                }
            }),
            "alwaystransparent" | "allwaystransparent" | "centerposition" | "center_on_anchor" => {
                if let (Some(flag), Some(icon)) = (read_bool(&text), element.fields.icon_mut()) {
                    match word {
                        "centerposition" => icon.center_position = flag,
                        "center_on_anchor" => icon.center_on_anchor = flag,
                        _ => icon.always_transparent = flag,
                    }
                }
            }
            "font" | "text" | "buttontext" | "buttonfont" => {}
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn button_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "position" | "size" => {
                let axes = self.int_point();
                let mut point = element.fields.button_mut().map_or([0; 2], |button| {
                    if word == "position" {
                        button.position
                    } else {
                        button.size
                    }
                });
                Self::store_point(element, word, axes, &mut point);
                if let Some(button) = element.fields.button_mut() {
                    if word == "position" {
                        button.position = point;
                    } else {
                        button.size = point;
                    }
                }
            }
            "bordersize" => {
                let axes = self.int_point();
                let mut point = [0; 2];
                Self::store_point(element, "borderSize", axes, &mut point);
                if let Some(button) = element.fields.button_mut() {
                    button.border_size = point;
                }
            }
            "scale" | "rotation" => {
                element.store(content_reader_parser::crt::scan_f32(&text), |fields, value| {
                    if let Some(button) = fields.button_mut() {
                        if word == "rotation" {
                            button.rotation = value;
                        } else {
                            button.scale = value;
                        }
                    }
                });
            }
            "frame" => element.store(content_reader_parser::crt::scan_i32(&text).stored(), |fields, value| {
                if let Some(button) = fields.button_mut() {
                    button.frame = value;
                }
            }),
            "format" => {
                if let (Some(format), Some(button)) =
                    (self.text_format(), element.fields.button_mut())
                {
                    button.format = format;
                }
            }
            "vertical_alignment" => {
                if let (Some(alignment), Some(button)) =
                    (self.vertical_alignment(), element.fields.button_mut())
                {
                    button.vertical_alignment = alignment;
                }
            }
            "centerposition" | "alwaystransparent" | "enabled" | "multiline" | "no_clicksound" => {
                if let (Some(flag), Some(button)) = (read_bool(&text), element.fields.button_mut())
                {
                    match word {
                        "centerposition" => button.center_position = flag,
                        "alwaystransparent" => button.always_transparent = flag,
                        "enabled" => button.enabled = flag,
                        "multiline" => button.multiline = flag,
                        _ => button.no_click_sound = flag,
                    }
                }
            }
            "spritetype" | "quadtexturesprite" | "font" | "buttonfont" | "origo" | "text"
            | "buttontext" | "parent" | "orientation" | "upsprite" | "downsprite"
            | "dissablesprite" | "oversprite" | "luaclick" | "callback_type"
            | "callback_argument" | "web_link" | "upsound" | "downsound" | "oversound"
            | "clicksound" | "shortcut" => {
                let value = read_string(&text);
                let Some(button) = element.fields.button_mut() else {
                    return Handled::Yes;
                };
                match word {
                    "spritetype" | "quadtexturesprite" => button.sprite = value,
                    "font" | "buttonfont" | "origo" => button.font = value,
                    "text" | "buttontext" => button.text = value,
                    "parent" => button.parent = value,
                    "orientation" => button.orientation_text = value,
                    "upsprite" => button.up_sprite = value,
                    "downsprite" => button.down_sprite = value,
                    "dissablesprite" => button.dissable_sprite = value,
                    "oversprite" => button.over_sprite = value,
                    "luaclick" => button.lua_click = value,
                    "callback_type" => button.callback_type = value,
                    "callback_argument" => button.callback_argument = value,
                    "web_link" => {
                        button.callback_type = b"web_link".to_vec();
                        button.callback_argument = value;
                    }
                    "upsound" => button.up_sound = value,
                    "downsound" => button.down_sound = value,
                    "oversound" => button.over_sound = value,
                    "clicksound" => {
                        if value == b"none" {
                            button.no_click_sound = true;
                            button.click_sound.clear();
                        } else {
                            button.click_sound = value;
                        }
                    }
                    _ => button.shortcuts.push(value),
                }
            }
            "open_browser" => self.unexpected(),
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn text_format(&self) -> Option<TextFormat> {
        if self.value.is_keyword("left") {
            Some(TextFormat::Left)
        } else if self.value.is_keyword("right") {
            Some(TextFormat::Right)
        } else if self.value.is_keyword("centre") || self.value.is_keyword("center") {
            Some(TextFormat::Center)
        } else {
            None
        }
    }

    fn vertical_alignment(&self) -> Option<VerticalAlignment> {
        if self.value.is_keyword("top") {
            Some(VerticalAlignment::Top)
        } else if self.value.is_keyword("center") || self.value.is_keyword("centre") {
            Some(VerticalAlignment::Center)
        } else if self.value.is_keyword("bottom") {
            Some(VerticalAlignment::Bottom)
        } else {
            None
        }
    }

    fn text_box_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "font" | "text" | "scrollbartype" => {
                let value = read_string(&text);
                if let Some(text_box) = element.fields.text_box_mut() {
                    match word {
                        "font" => text_box.font = value,
                        "text" => text_box.text = value,
                        _ => text_box.scrollbar_type = value,
                    }
                }
            }
            "texturefile" => {}
            "size" => {
                let _ = self.point_tokens();
            }
            "position" | "bordersize" => {
                let axes = self.int_point();
                let key = if word == "position" {
                    "position"
                } else {
                    "borderSize"
                };
                let mut point = element.fields.text_box_mut().map_or([0; 2], |text_box| {
                    if word == "position" {
                        text_box.position
                    } else {
                        text_box.border_size
                    }
                });
                Self::store_point(element, key, axes, &mut point);
                if let Some(text_box) = element.fields.text_box_mut() {
                    if word == "position" {
                        text_box.position = point;
                    } else {
                        text_box.border_size = point;
                    }
                }
            }
            "format" => {
                if let (Some(format), Some(text_box)) =
                    (self.text_format(), element.fields.text_box_mut())
                {
                    text_box.format = format;
                }
            }
            "vertical_alignment" => {
                if let (Some(alignment), Some(text_box)) =
                    (self.vertical_alignment(), element.fields.text_box_mut())
                {
                    text_box.vertical_alignment = alignment;
                }
            }
            "maxwidth" | "maxheight" => {
                element.store(content_reader_parser::crt::scan_u32(&text).stored(), |fields, value| {
                    if let Some(text_box) = fields.text_box_mut() {
                        if word == "maxwidth" {
                            text_box.max_width = value;
                        } else {
                            text_box.max_height = value;
                        }
                    }
                });
            }
            "orientation" => element.orientation_text = Some(read_string(&text)),
            "fixedsize"
            | "truncate"
            | "multiline"
            | "alwaystransparent"
            | "allwaystransparent"
            | "ignore_special_chars" => {
                if let (Some(flag), Some(text_box)) =
                    (read_bool(&text), element.fields.text_box_mut())
                {
                    match word {
                        "fixedsize" => text_box.fixed_size = flag,
                        "truncate" => text_box.truncate = flag,
                        "multiline" => text_box.multiline = flag,
                        "ignore_special_chars" => text_box.ignore_special_chars = flag,
                        _ => text_box.always_transparent = flag,
                    }
                }
            }
            "text_color_code" => {
                if let ([code], Some(text_box)) = (
                    content_reader_parser::parser::c_string(&text),
                    element.fields.text_box_mut(),
                ) {
                    text_box.text_color_code = *code;
                }
            }
            "context_aware_text" | "bound_text" => {
                let unset = element
                    .fields
                    .text_box_mut()
                    .is_some_and(|text_box| text_box.bound_text.is_empty());
                if unset {
                    let bound = self.bound_localization();
                    if let Some(text_box) = element.fields.text_box_mut() {
                        text_box.bound_text = bound;
                        if word == "context_aware_text" {
                            text_box.context_aware = true;
                        }
                    }
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn grid_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "position" => {
                let mut position = element
                    .fields
                    .grid_mut()
                    .map(|grid| grid.position)
                    .unwrap_or_default();
                self.position(&mut element.refused, "position", &mut position);
                if let Some(grid) = element.fields.grid_mut() {
                    grid.position = position;
                }
            }
            "size" | "slotsize" => {
                let slot = word == "slotsize";
                let mut size = element
                    .fields
                    .grid_mut()
                    .map(|grid| if slot { grid.slot_size } else { grid.size })
                    .unwrap_or_default();
                self.size(&mut element.refused, word, &mut size);
                if let Some(grid) = element.fields.grid_mut() {
                    if slot {
                        grid.slot_size = size;
                    } else {
                        grid.size = size;
                    }
                }
            }
            "format" => {
                let value = GridFormat::parse(content_reader_parser::parser::c_string(&text));
                if let Some(grid) = element.fields.grid_mut() {
                    grid.format = value;
                }
            }
            "orientation" => {
                let value = match Orientation::from_keyword(&self.value) {
                    Some(orientation) => orientation as i32,
                    None if self.value.quoted() => {
                        match content_reader_parser::parser::c_string(&text)
                            .to_ascii_lowercase()
                            .as_slice()
                        {
                            b"upper_right" => 1,
                            b"lower_left" => 2,
                            b"lower_right" => 3,
                            b"center" | b"center_left" | b"center_right" => 4,
                            b"center_up" => 7,
                            b"center_down" => 8,
                            _ => 0,
                        }
                    }
                    None => 0,
                };
                if let Some(grid) = element.fields.grid_mut() {
                    grid.orientation = value;
                }
            }
            "add_horizontal" => {
                if let (Some(flag), Some(grid)) = (read_bool(&text), element.fields.grid_mut()) {
                    grid.add_horizontal = flag;
                }
            }
            "max_slots_horizontal" | "max_slots_vertical" => {
                let index = usize::from(word == "max_slots_vertical");
                element.store(content_reader_parser::crt::scan_i32(&text).stored(), |fields, value| {
                    if let Some(grid) = fields.grid_mut() {
                        grid.max_slots[index] = value;
                    }
                });
            }
            "max_slots" => {
                let mut written = Vec::new();
                let result = self.components(|index, value| written.push((index, value as u32)));
                if let Some(grid) = element.fields.grid_mut()
                    && let Err(reason) = spill::spill(
                        &mut GridBytes(grid),
                        "integer component reader",
                        0x1B8,
                        written,
                    )
                {
                    element.refused.refuse("max_slots", reason);
                }
                if let Err(reason) = result {
                    self.poison(&mut element.refusals, element.node, "max_slots", &reason);
                }
            }
            "padding" => {
                let mut padding = element
                    .fields
                    .grid_mut()
                    .map(|grid| grid.padding)
                    .unwrap_or_default();
                self.margin(&mut element.refused, "padding", &mut padding);
                if let Some(grid) = element.fields.grid_mut() {
                    grid.padding = padding;
                }
            }
            "background" => {
                if !std::mem::replace(&mut element.grid_background, true) {
                    return Handled::Opens(Nested {
                        build: Build::Button,
                        spelled: "background".into(),
                        attach: Attach::GridBackground,
                    });
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn list_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "parent" => {
                if let Some(list) = element.fields.list_mut() {
                    list.parent = read_string(&text);
                }
            }
            "position" | "size" => {
                let axes = self.int_point();
                let mut point = element.fields.list_mut().map_or([0; 2], |list| {
                    if word == "position" {
                        list.position
                    } else {
                        list.size
                    }
                });
                Self::store_point(element, word, axes, &mut point);
                if let Some(list) = element.fields.list_mut() {
                    if word == "position" {
                        list.position = point;
                    } else {
                        list.size = point;
                    }
                }
            }
            "format" => {
                let value = [
                    ("center", ListFormat::Center),
                    ("centre", ListFormat::Center),
                    ("right", ListFormat::Right),
                    ("left", ListFormat::Left),
                    ("up", ListFormat::Up),
                    ("down", ListFormat::Down),
                ]
                .iter()
                .find(|(keyword, _)| self.value.is_keyword(keyword))
                .map(|(_, value)| *value);
                match (value, element.fields.list_mut()) {
                    (Some(value), Some(list)) => list.format = value,
                    (None, _) => self.unexpected(),
                    _ => {}
                }
            }
            "orientation" => {
                if let Some(list) = element.fields.list_mut() {
                    list.orientation = Orientation::parse_gui(&read_string(&text));
                }
            }
            "spacing" => element.store(content_reader_parser::crt::scan_i32(&text).stored(), |fields, value| {
                if let Some(list) = fields.list_mut() {
                    list.spacing = value;
                }
            }),
            "first_on_top" => {
                if let (Some(flag), Some(list)) = (read_bool(&text), element.fields.list_mut()) {
                    list.first_on_top = flag;
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn position_type_key(&mut self, element: &mut Building, word: &str) -> Handled {
        match word {
            "position" => {
                let axes = self.int_point();
                let mut point = match &element.fields {
                    Fields::PositionType(fields) => fields.position,
                    _ => [0; 2],
                };
                Self::store_point(element, "position", axes, &mut point);
                if let Fields::PositionType(fields) = &mut element.fields {
                    fields.position = point;
                }
            }
            "scale" => {
                let value = content_reader_parser::crt::scan_f32(&self.value.text);
                element.store(value, |fields, value| {
                    if let Fields::PositionType(fields) = fields {
                        fields.scale = value;
                    }
                });
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn list_box_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        let Fields::ListBox(fields) = &mut element.fields else {
            return Handled::No;
        };
        match word {
            "offset" | "size" | "position" | "bordersize" => {
                let mut point = match word {
                    "offset" => fields.offset,
                    "size" => fields.size,
                    "position" => fields.position,
                    _ => fields.border_size,
                };
                let axes = self.int_point();
                Self::store_point(element, word, axes, &mut point);
                if let Fields::ListBox(fields) = &mut element.fields {
                    match word {
                        "offset" => fields.offset = point,
                        "size" => fields.size = point,
                        "position" => fields.position = point,
                        _ => fields.border_size = point,
                    }
                }
            }
            "parent" | "scrollbartype" | "background" | "dropdown" | "heading" | "orientation"
            | "membername" => {
                let value = read_string(&text);
                match word {
                    "parent" => fields.parent = value,
                    "scrollbartype" => fields.scrollbar_type = value,
                    "background" => fields.background = value,
                    "dropdown" => fields.drop_down = value,
                    "heading" => fields.heading = value,
                    "orientation" => fields.orientation = value,
                    _ => fields.member_name = value,
                }
            }
            "priority" => {
                if let Some(value) = content_reader_parser::crt::scan_f32(&text) {
                    fields.priority = Some(value);
                }
            }
            "horizontal" => {
                fields.horizontal = content_reader_parser::crt::scan_i32(&text)
                    .stored()
                    .unwrap_or(fields.horizontal)
            }
            "step" | "spacing" => {
                fields.spacing = content_reader_parser::crt::scan_i32(&text)
                    .stored()
                    .unwrap_or(fields.spacing)
            }
            "alwaystransparent" | "allwaystransparent" => {
                fields.always_transparent = read_bool(&text).unwrap_or(fields.always_transparent);
            }
            "clipping" => fields.clipping = read_bool(&text).unwrap_or(fields.clipping),
            "scrollbar_side" => {
                let side = read_string(&text);
                if side.eq_ignore_ascii_case(b"left") {
                    fields.scrollbar_side = 0;
                } else if side.eq_ignore_ascii_case(b"right") {
                    fields.scrollbar_side = 1;
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn text_sprite_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "position" | "bordersize" => {
                let axes = self.int_point();
                let mut point = match &element.fields {
                    Fields::TextSprite(fields) if word == "position" => fields.position,
                    Fields::TextSprite(fields) => fields.border_size,
                    _ => [0; 2],
                };
                Self::store_point(element, word, axes, &mut point);
                if let Fields::TextSprite(fields) = &mut element.fields {
                    if word == "position" {
                        fields.position = point;
                    } else {
                        fields.border_size = point;
                    }
                }
            }
            "textblock" => {
                let block = self.text_block();
                if let Fields::TextSprite(fields) = &mut element.fields {
                    fields.text_blocks.push(block);
                }
            }
            "orientation" => element.orientation_text = Some(read_string(&text)),
            _ => {
                let Fields::TextSprite(fields) = &mut element.fields else {
                    return Handled::No;
                };
                match word {
                    "font" => fields.font = read_string(&text),
                    "texturefile" => fields.texture = read_string(&text),
                    "text" => fields.text = read_string(&text),
                    "format" => {
                        if self.value.is_keyword("centre") {
                            fields.format = 2;
                        } else if self.value.is_keyword("right") {
                            fields.format = 1;
                        } else if self.value.is_keyword("left") {
                            fields.format = 0;
                        }
                    }
                    "maxheight" => {
                        fields.max_height = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(fields.max_height);
                    }
                    "maxwidth" => {
                        fields.max_width = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(fields.max_width)
                    }
                    "fixedsize" => {
                        fields.fixed_size = read_bool(&text).unwrap_or(fields.fixed_size)
                    }
                    _ => return Handled::No,
                }
            }
        }
        Handled::Yes
    }

    fn text_block(&mut self) -> TextBlock {
        let mut block = TextBlock::default();
        self.read_text_block(&mut block);
        block
    }

    pub(super) fn read_text_block(&mut self, block: &mut TextBlock) {
        while self.next_key() {
            let key = self.key.clone();
            let text = self.value.text.clone();
            match keyword_of(&key) {
                Some("font") => block.font = read_string(&text),
                Some("text") => block.text = read_string(&text),
                Some("size") => {
                    let axes = self.int_point();
                    block.size = point_or_zero(axes);
                }
                Some("position") => {
                    let axes = self.int_point();
                    let new = point_or_zero(axes);
                    for ((size, old), new) in block.size.iter_mut().zip(block.position).zip(new) {
                        *size = size.wrapping_add(old.wrapping_sub(new));
                    }
                    block.position = new;
                }
                Some("color") => self.color(&mut block.color),
                Some("format") => {
                    if self.value.is_keyword("centre") {
                        block.format = 2;
                    } else if self.value.is_keyword("right") {
                        block.format = 1;
                    } else if self.value.is_keyword("left") {
                        block.format = 0;
                    }
                }
                _ => {}
            }
        }
    }

    fn edit_box_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        match word {
            "size" | "position" | "bordersize" => {
                let start = match word {
                    "position" => 0x150,
                    "size" => 0x158,
                    _ => 0x160,
                };
                let mut written = Vec::new();
                let result = self.components(|index, value| written.push((index, value as u32)));
                if let Fields::EditBox(fields) = &mut element.fields
                    && let Err(reason) = spill::spill(
                        &mut EditBoxBytes(fields),
                        "integer component reader",
                        start,
                        written,
                    )
                {
                    element.refused.refuse(word, reason);
                }
                if let Err(reason) = result {
                    self.poison(&mut element.refusals, element.node, word, &reason);
                }
            }
            "cursor" => {
                let axes = self.int_point();
                let mut point = [0; 2];
                Self::store_point(element, "cursor", axes, &mut point);
                if let Fields::EditBox(fields) = &mut element.fields {
                    fields.slots[6] = point[0];
                    fields.slots[7] = point[1];
                }
            }
            _ => {
                let Fields::EditBox(fields) = &mut element.fields else {
                    return Handled::No;
                };
                match word {
                    "font" => fields.font = read_string(&text),
                    "texturefile" => fields.texture = read_string(&text),
                    "text" => fields.text = read_string(&text),
                    "orientation" => {
                        fields.slots[8] = Orientation::parse_gui(&read_string(&text)).gui_value();
                    }
                    "is_multiline" => {
                        if let Some(flag) = read_bool(&text) {
                            let mut bytes = fields.slots[10].to_le_bytes();
                            bytes[0] = u8::from(flag);
                            fields.slots[10] = i32::from_le_bytes(bytes);
                        }
                    }
                    "ignore_tab_navigation"
                    | "instanttextboxtype"
                    | "use_special_chars"
                    | "only_numbers"
                    | "ignore_enter" => {
                        let bit = match word {
                            "ignore_tab_navigation" => 0x1,
                            "instanttextboxtype" => 0x2,
                            "use_special_chars" => 0x4,
                            "only_numbers" => 0x8,
                            _ => 0x10,
                        };
                        let set = read_bool(&text).unwrap_or(false);
                        let mut bytes = fields.slots[9].to_le_bytes();
                        if set {
                            bytes[0] |= bit;
                        } else {
                            bytes[0] &= !bit;
                        }
                        fields.slots[9] = i32::from_le_bytes(bytes);
                    }
                    _ => return Handled::No,
                }
            }
        }
        Handled::Yes
    }

    fn scrollbar_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        if matches!(word, "guibuttontype" | "buttontype" | "icontype")
            && let Some(reason) = element.factory.clone()
        {
            element.refused.refuse(word, reason);
        }
        match word {
            "guibuttontype" => return Handled::child(Build::GuiButton, &self.key),
            "buttontype" => return Handled::child(Build::Button, &self.key),
            "icontype" => return Handled::child(Build::Icon, &self.key),
            "position" | "size" | "bordersize" => {
                let axes = self.int_point();
                let mut point = match &element.fields {
                    Fields::Scrollbar(bar) => match word {
                        "position" => bar.position,
                        "size" => bar.size,
                        _ => bar.border_size,
                    },
                    _ => [0; 2],
                };
                Self::store_point(element, word, axes, &mut point);
                if let Fields::Scrollbar(bar) = &mut element.fields {
                    match word {
                        "position" => bar.position = point,
                        "size" => bar.size = point,
                        _ => bar.border_size = point,
                    }
                }
            }
            "slider_pos_shift" => {
                let mut written = Vec::new();
                let result = self.components(|index, value| written.push((index, value as u32)));
                if let Fields::Scrollbar(bar) = &mut element.fields {
                    let mut bytes = ScrollbarBytes {
                        bar,
                        factory: &mut element.factory,
                    };
                    if let Err(reason) =
                        spill::spill(&mut bytes, "integer component reader", 0x1F0, written)
                    {
                        element.refused.refuse("slider_pos_shift", reason);
                    }
                }
                if let Err(reason) = result {
                    self.poison(&mut element.refusals, element.node, word, &reason);
                }
            }
            _ => {
                let Fields::Scrollbar(bar) = &mut element.fields else {
                    return Handled::No;
                };
                match word {
                    "maxvalue" | "minvalue" | "stepsize" | "startvalue" | "priority" => {
                        if let Some(value) = content_reader_parser::crt::scan_f32(&text) {
                            let slot = match word {
                                "maxvalue" => 2,
                                "minvalue" => 3,
                                "stepsize" => 4,
                                "startvalue" => 5,
                                _ => 6,
                            };
                            bar.slots[slot] = value.to_bits();
                            if slot == 3 {
                                bar.min_value_set = true;
                            }
                        }
                    }
                    "horizontal" => {
                        bar.horizontal = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(bar.horizontal)
                    }
                    "scroll_speed" => {
                        bar.scroll_speed = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(bar.scroll_speed);
                    }
                    "lockable" => bar.lockable = read_bool(&text).unwrap_or(bar.lockable),
                    "drag" => bar.drag = read_bool(&text).unwrap_or(bar.drag),
                    "userangelimit" => {
                        bar.use_range_limit = read_bool(&text).unwrap_or(bar.use_range_limit);
                    }
                    "rangelimitmin" => {
                        bar.range_limit_min =
                            content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.range_limit_min);
                    }
                    "rangelimitmax" => {
                        bar.range_limit_max =
                            content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.range_limit_max);
                    }
                    "slider" => bar.slider = read_string(&text),
                    "track" => bar.track = read_string(&text),
                    "rightbutton" => bar.right_button = read_string(&text),
                    "leftbutton" => bar.left_button = read_string(&text),
                    "rangelimitminicon" => bar.range_limit_min_icon = read_string(&text),
                    "rangelimitmaxicon" => bar.range_limit_max_icon = read_string(&text),
                    "parent" => bar.parent = read_string(&text),
                    _ => return Handled::No,
                }
            }
        }
        Handled::Yes
    }

    fn extended_scrollbar_key(&mut self, element: &mut Building, word: &str) -> Handled {
        let text = self.value.text.clone();
        let parts = [
            ("background", Part::ScrollbarBackground),
            ("track", Part::Track),
            ("slider", Part::Slider),
            ("increasebutton", Part::IncreaseButton),
            ("decreasebutton", Part::DecreaseButton),
        ];
        if let Some((_, part)) = parts.iter().find(|(name, _)| *name == word) {
            let Fields::ExtendedScrollbar(bar) = &element.fields else {
                return Handled::No;
            };
            let taken = match part {
                Part::ScrollbarBackground => bar.background.is_some(),
                Part::Track => bar.track.is_some(),
                Part::Slider => bar.slider.is_some(),
                Part::IncreaseButton => bar.increase_button.is_some(),
                _ => bar.decrease_button.is_some(),
            };
            if taken {
                return Handled::Yes;
            }
            return Handled::part(Build::Button, &self.key.clone(), Attach::Part(*part));
        }
        match word {
            "position" => {
                let mut position = match &element.fields {
                    Fields::ExtendedScrollbar(bar) => bar.position,
                    _ => Position::default(),
                };
                self.position(&mut element.refused, word, &mut position);
                if let Fields::ExtendedScrollbar(bar) = &mut element.fields {
                    bar.position = position;
                }
            }
            "size" | "tilesize" => {
                let mut size = match &element.fields {
                    Fields::ExtendedScrollbar(bar) if word == "size" => bar.size,
                    Fields::ExtendedScrollbar(bar) => bar.tile_size,
                    _ => Size::default(),
                };
                self.size(&mut element.refused, word, &mut size);
                if let Fields::ExtendedScrollbar(bar) = &mut element.fields {
                    if word == "size" {
                        bar.size = size;
                    } else {
                        bar.tile_size = size;
                    }
                }
            }
            "orientation" | "origo" => {
                let value = Orientation::from_keyword(&self.value).unwrap_or_default();
                if let Fields::ExtendedScrollbar(bar) = &mut element.fields {
                    if word == "orientation" {
                        bar.orientation = value;
                    } else {
                        bar.origo = value;
                    }
                }
            }
            _ => {
                let Fields::ExtendedScrollbar(bar) = &mut element.fields else {
                    return Handled::No;
                };
                match word {
                    "maxvalue" => {
                        bar.max_value = content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.max_value)
                    }
                    "minvalue" => {
                        bar.min_value = content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.min_value)
                    }
                    "stepsize" => {
                        bar.step_size = content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.step_size)
                    }
                    "startvalue" => {
                        bar.start_value =
                            content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.start_value);
                    }
                    "smooth_scrolling" => {
                        bar.smooth_scrolling =
                            content_reader_parser::fixed5::script_fixed5(&text).unwrap_or(bar.smooth_scrolling);
                    }
                    "horizontal" => bar.horizontal = read_bool(&text).unwrap_or(bar.horizontal),
                    "lockable" => bar.lockable = read_bool(&text).unwrap_or(bar.lockable),
                    "drag" => bar.drag = read_bool(&text).unwrap_or(bar.drag),
                    "clickonly" => bar.click_only = read_bool(&text).unwrap_or(bar.click_only),
                    "settrackframeonchange" => {
                        bar.set_track_frame_on_change =
                            read_bool(&text).unwrap_or(bar.set_track_frame_on_change);
                    }
                    _ => return Handled::No,
                }
            }
        }
        Handled::Yes
    }

    fn browser_key(&mut self, element: &mut Building, word: &str) -> Handled {
        match word {
            "position" | "size" => {
                let mut written = Vec::new();
                let result = self.components(|index, value| written.push((index, value as u32)));
                let curve = element.build == Build::CurveGraph;
                if let Fields::Browser(fields) = &mut element.fields {
                    let mut bytes = BrowserBytes {
                        fields: &mut *fields,
                        sprite: &mut element.sprite_string,
                        curve,
                    };
                    let start = bytes.base() + if word == "position" { 0 } else { 8 };
                    let written =
                        spill::spill(&mut bytes, "integer component reader", start, written);
                    match written.and_then(|()| {
                        element.sprite_string.text().map_err(|reason| {
                            format!("leaves spriteType (its string) unreadable: {reason}")
                        })
                    }) {
                        Ok(text) => fields.sprite = text,
                        Err(reason) => element.refused.refuse(word, reason),
                    }
                }
                if let Err(reason) = result {
                    self.poison(&mut element.refusals, element.node, word, &reason);
                }
            }
            "spritetype" => {
                let value = read_string(&self.value.text);
                let assigned = element
                    .sprite_string
                    .assign(&value)
                    .and_then(|()| element.sprite_string.text());
                match (assigned, &mut element.fields) {
                    (Ok(text), Fields::Browser(fields)) => fields.sprite = text,
                    (Err(reason), _) => element.refused.refuse("spritetype", reason),
                    _ => {}
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn window_key(&mut self, element: &mut Building, word: &str) -> Handled {
        if let Some(build) = window_child(word) {
            return Handled::child(build, &self.key);
        }
        let text = self.value.text.clone();
        match word {
            "position" | "show_position" | "hide_position" => {
                let axes = self.int_point();
                let mut point = match &element.fields {
                    Fields::Window(window) => match word {
                        "position" => window.position,
                        "show_position" => window.show_position,
                        _ => window.hide_position,
                    },
                    _ => [0; 2],
                };
                Self::store_point(element, word, axes, &mut point);
                if let Fields::Window(window) = &mut element.fields {
                    match word {
                        "position" => window.position = point,
                        "show_position" => window.show_position = point,
                        _ => window.hide_position = point,
                    }
                }
            }
            "size" => self.window_size(element),
            _ => {
                let Fields::Window(window) = &mut element.fields else {
                    return Handled::No;
                };
                match word {
                    "orientation" => {
                        window.orientation = Orientation::parse_gui(&read_string(&text));
                    }
                    "animation_type" => {
                        window.animation_type =
                            match read_string(&text).to_ascii_lowercase().as_slice() {
                                b"accelerated" => 0,
                                b"smoothstep" => 2,
                                b"linear" => 3,
                                _ => 1,
                            };
                    }
                    "animation_time" => {
                        window.animation_time = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(window.animation_time);
                    }
                    "moveable" => {
                        window.moveable = content_reader_parser::crt::scan_i32(&text)
                            .stored()
                            .unwrap_or(window.moveable)
                    }
                    "click_to_front" => {
                        window.click_to_front = read_bool(&text).unwrap_or(window.click_to_front);
                    }
                    "fullscreen" => {
                        window.full_screen = read_bool(&text).unwrap_or(window.full_screen);
                    }
                    "priority" => {
                        if let Some(value) = content_reader_parser::crt::scan_f32(&text) {
                            window.priority = Some(value);
                        }
                    }
                    "parent" => window.parent = read_string(&text),
                    "background" => window.background = read_string(&text),
                    "verticalscrollbar" => window.vertical_scrollbar = read_string(&text),
                    "horizontalscrollbar" => window.horizontal_scrollbar = read_string(&text),
                    "horizontalborder" => window.horizontal_border = read_string(&text),
                    "verticalborder" => window.vertical_border = read_string(&text),
                    "upsound" => window.up_sound = read_string(&text),
                    "downsound" => window.down_sound = read_string(&text),
                    _ => return Handled::No,
                }
            }
        }
        Handled::Yes
    }

    fn window_size(&mut self, element: &mut Building) {
        let values = self.point_tokens();
        let Fields::Window(window) = &mut element.fields else {
            return;
        };
        let less = if window.show_position == [0, 0] {
            window.position
        } else {
            window.show_position
        };
        let mut local: Option<i32> = None;
        for (axis, token) in values.iter().enumerate() {
            let Some(token) = token else {
                break;
            };
            if token.is_keyword("max_resolution") {
                window.size[axis] = WindowExtent::MaxResolution { less: less[axis] };
            } else if token.is_number() {
                if let Some(value) = content_reader_parser::crt::scan_i32(&token.text).stored() {
                    local = Some(value);
                }
                match local {
                    Some(value) => window.size[axis] = WindowExtent::Pixels(value),
                    None => element.refused.refuse(
                        "size",
                        "scans nothing of a number, so the window size reader stores its \
                         uninitialised local: an undefined read"
                            .into(),
                    ),
                }
            }
        }
    }

    fn finish_element(&mut self, mut element: Building) -> Element {
        if element.named && element.name.is_empty() {
            element.name = format!("unnamed_gui_{}", self.unnamed).into_bytes();
            self.unnamed = self.unnamed.wrapping_add(1);
        }
        self.labels[element.node].text = if element.name.is_empty() {
            format!("<{}>", element.spelled)
        } else {
            spelling(&element.name)
        };
        if let Some(text) = element.orientation_text.take() {
            let orientation = Orientation::parse_gui(&text);
            match &mut element.fields {
                Fields::TextBox(text_box) => text_box.orientation = orientation,
                Fields::TextSprite(text_sprite) => text_sprite.orientation = orientation,
                _ => {}
            }
        }
        if let Some(inner) = element.inner.take() {
            let inner = self.finish_element(*inner);
            if let Fields::DropDown(fields) = &mut element.fields {
                *fields.inner = inner;
            }
        }
        for (key, reason) in std::mem::take(&mut element.refused.by_key) {
            record(
                &mut element.refusals,
                element.node,
                &key,
                reason,
                Cause::Unread,
            );
        }
        if let Some(poison) = &self.poison
            && !element.refusals.contains(poison)
        {
            element.refusals.push(poison.clone());
        }
        Element {
            kind: element.build.kind(),
            spelling: element.spelled,
            path: String::new(),
            name: element.name,
            hide: element.hide,
            common: element.common,
            fields: element.fields,
            children: element.children,
            refusals: element.refusals,
            node: element.node,
        }
    }
}

fn point_or_zero(axes: [Axis; 2]) -> [i32; 2] {
    axes.map(|axis| match axis {
        Axis::Value(value) => value,
        Axis::Untouched | Axis::Undefined => 0,
    })
}

fn attach_to(parent: &mut Building, attach: Attach, child: Element) {
    match attach {
        Attach::Root => {}
        Attach::Background => {
            if let Fields::Container(container) = &mut parent.fields {
                container.backgrounds.push(child);
            }
        }
        Attach::GridBackground => {
            if let Fields::Grid(grid) = &mut parent.fields {
                grid.background = Some(Box::new(child));
            }
        }
        Attach::InnerChild | Attach::InnerBackground => {
            if let Some(inner) = parent.inner.as_mut() {
                let attach = if attach == Attach::InnerChild {
                    Attach::Child
                } else {
                    Attach::Background
                };
                attach_to(inner, attach, child);
            }
        }
        Attach::Part(part) => match &mut parent.fields {
            Fields::ExtendedScrollbar(bar) => {
                let slot = match part {
                    Part::ScrollbarBackground => &mut bar.background,
                    Part::Track => &mut bar.track,
                    Part::Slider => &mut bar.slider,
                    Part::IncreaseButton => &mut bar.increase_button,
                    _ => &mut bar.decrease_button,
                };
                *slot = Some(Box::new(child));
            }
            Fields::DropDown(drop_down) => {
                if part == Part::ExpandButton {
                    drop_down.expand_button = Some(Box::new(child));
                } else {
                    drop_down.expanded_window = Some(Box::new(child));
                }
            }
            _ => {}
        },
        Attach::Child => match parent.build {
            Build::Window => {
                if let Fields::Window(window) = &mut parent.fields
                    && matches!(child.kind, Kind::GuiButton | Kind::MultiSpriteButton)
                    && child.name_bytes() == window.background.as_slice()
                {
                    window.unlisted.push(parent.children.len());
                }
                parent.children.push(child);
            }
            Build::Scrollbar => {
                parent
                    .children
                    .retain(|own| !own.is_named(child.name_bytes()));
                parent.children.push(child);
            }
            _ => {
                if parent
                    .children
                    .iter()
                    .any(|own| own.is_named(child.name_bytes()))
                {
                    for refusal in &child.refusals {
                        if refusal.cause == Cause::Unaligned {
                            record(
                                &mut parent.refusals,
                                refusal.node,
                                &refusal.key,
                                refusal.reason.clone(),
                                refusal.cause,
                            );
                        }
                    }
                } else {
                    parent.children.push(child);
                }
            }
        },
    }
}

fn spelling(key: &[u8]) -> String {
    String::from_utf8_lossy(key).into_owned()
}

fn container(element: &mut Building) -> &mut Container {
    match &mut element.fields {
        Fields::Container(container) => container,
        _ => unreachable!("only a container's keys reach its fields"),
    }
}

struct GridBytes<'a>(&'a mut Grid);

impl Layout for GridBytes<'_> {
    fn end(&self) -> (usize, String) {
        spill::heap_end(0x1D0, "container reader")
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let grid = &mut *self.0;
        match offset {
            0x1B8 | 0x1BC => grid.max_slots[(offset - 0x1B8) / 4] = bits as i32,
            0x1C0 => grid.format = bits as i32,
            0x1C4 => grid.orientation = bits as i32,
            0x1C8 => grid.add_horizontal = bits.to_le_bytes()[0] != 0,
            _ => {}
        }
        Ok(())
    }
}

struct EditBoxBytes<'a>(&'a mut EditBox);

impl Layout for EditBoxBytes<'_> {
    fn end(&self) -> (usize, String) {
        spill::heap_end(0x180, "container reader")
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        self.0.slots[(offset - 0x150) / 4] = bits as i32;
        Ok(())
    }
}

struct ScrollbarBytes<'a> {
    bar: &'a mut Scrollbar,
    factory: &'a mut Option<String>,
}

impl Layout for ScrollbarBytes<'_> {
    fn end(&self) -> (usize, String) {
        spill::heap_end(0x2A0, "container reader")
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        match offset {
            0x1F0..=0x20C => {
                let slot = (offset - 0x1F0) / 4;
                self.bar.slots[slot] = bits;
                if slot == 3 {
                    self.bar.min_value_set = true;
                }
            }
            0x210 | 0x214 => {}
            0x218 | 0x21C => {
                self.factory.get_or_insert_with(|| {
                    "follows a slider_pos_shift whose eleventh or twelfth component wrote the \
                     child factory pointer, which the scrollbar reader calls through for \
                     this child: a heap address the replay does not hold"
                        .into()
                });
            }
            0x220..=0x23C => {
                return Err(
                    "the child map (the name tree's virtual-function table at the image's \
                     load address and its root), which the scrollbar post-load pass walks in \
                     every post-load pass: addresses the replay does not hold"
                        .into(),
                );
            }
            0x240 | 0x244 if bits != 0 => {
                return Err(
                    "the child map's batch queue, which the scrollbar destructor walks \
                     and frees: bits that are no block"
                        .into(),
                );
            }
            0x240..=0x25C => {}
            0x260 => self.bar.horizontal = bits as i32,
            0x264 => {}
            0x268 | 0x26C | 0x278 | 0x27C => {
                return Err(
                    "the listed field ids' vector (its data and allocator pointers), which the \
                     scrollbar destructor frees through that allocator: a heap or load address \
                     the replay does not hold"
                        .into(),
                );
            }
            0x270 => {}
            0x274 => self.bar.listed = bits as i32,
            0x280 => {
                self.bar.lockable = bits.to_le_bytes()[0] != 0;
                self.bar.drag = bits.to_le_bytes()[1] != 0;
            }
            0x288 | 0x28C | 0x290 | 0x294 => {
                let field = if offset < 0x290 {
                    &mut self.bar.range_limit_min
                } else {
                    &mut self.bar.range_limit_max
                };
                let mut value = field.to_le_bytes();
                let at = offset % 8;
                value[at..at + 4].copy_from_slice(&bits.to_le_bytes());
                *field = i64::from_le_bytes(value);
            }
            0x298 => self.bar.use_range_limit = bits.to_le_bytes()[0] != 0,
            0x29C => self.bar.scroll_speed = bits as i32,
            _ => {}
        }
        Ok(())
    }
}

struct BrowserBytes<'a> {
    fields: &'a mut Browser,
    sprite: &'a mut spill::CxxString,
    curve: bool,
}

impl BrowserBytes<'_> {
    fn base(&self) -> usize {
        if self.curve { 0xF0 } else { 0x100 }
    }
}

impl Layout for BrowserBytes<'_> {
    fn end(&self) -> (usize, String) {
        spill::heap_end(self.base() + 0x30, "GUI object factory")
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let relative = offset - self.base();
        if relative < 0x10 {
            self.fields.slots[relative / 4] = bits as i32;
            return Ok(());
        }
        self.sprite.write(relative - 0x10, bits)
    }
}
