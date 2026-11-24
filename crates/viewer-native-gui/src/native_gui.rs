
#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap};
use std::{borrow::Cow, cell::RefCell};

use content_reader_gui::gui::native::{self, Animation, Coordinate, Element, Fields, GridFormat, Kind, ListFormat, Orientation, TextFormat, VerticalAlignment, fonts::TextColors};
use content_reader_localisation::localisation::Localisation;

pub mod animation;
pub mod fixed;
mod hit;
pub mod lifecycle;
pub mod pointer;
pub mod text_sprites;
pub mod widgets;

pub use animation::{Animated, AnimationRecord};
pub use fixed::fixed_window_layout;
pub use hit::{AlphaMask, HitGeometry};

use super::{
    native_effects::{Blend, Effect, FixtureTechnique, Sampler, Settles, Settling, Technique},
    native_text::{self, Limits, Piece, Placement},
    sprites::{Batch, SpriteSheet},
};

#[cfg(any(test, feature = "test-support"))]
use super::sprites::{Exact, Quad};

pub fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub const FALLBACK_SPRITE: &[u8] = content_reader_gui::gui::native::resources::FALLBACK_SPRITE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub path: String,
    pub kind: RefusalKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefusalKind {
    Reader,
    UndefinedRead,
    NullRead,
    TypeConfusion,
    DivideByZero,
    Hang,
    MissingInput,
    CustomEffect,
    MissingResource,
    NoParent,
}

impl RefusalKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Reader => "gui_reader",
            Self::UndefinedRead => "undefined_read",
            Self::NullRead => "null_read",
            Self::TypeConfusion => "type_confusion",
            Self::DivideByZero => "divide_by_zero",
            Self::Hang => "hang",
            Self::MissingInput => "missing_input",
            Self::CustomEffect => "custom_effect",
            Self::MissingResource => "missing_resource",
            Self::NoParent => "no_parent",
        }
    }
}

impl Refusal {
    fn new(path: &str, kind: RefusalKind, detail: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            kind,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.path, self.kind.name(), self.detail)
    }
}

pub type Refusals = Vec<Refusal>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub kind: DiagnosticKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticKind {
    MissingFallback,
    UndefinedTextIcon,
    NullTextFlag,
    EmptyTextBake,
    NoTexture,
    MissingScrollbar,
    UndecodableText,
    NullFontPage,
    PoolGrowth,
}

impl DiagnosticKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::MissingFallback => "missing_fallback_sprite",
            Self::UndefinedTextIcon => "undefined_text_icon",
            Self::NullTextFlag => "null_text_flag",
            Self::EmptyTextBake => "empty_text_bake",
            Self::NoTexture => "sprite_without_texture",
            Self::MissingScrollbar => "missing_scrollbar",
            Self::UndecodableText => "undecodable_text",
            Self::NullFontPage => "null_font_page",
            Self::PoolGrowth => "map_icon_pool_growth",
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts(pub BTreeMap<&'static str, u64>);

#[cfg(any(test, feature = "test-support"))]
impl Counts {
    pub fn refused(&mut self, refusals: &[Refusal]) {
        for refusal in refusals {
            *self.0.entry(refusal.kind.name()).or_default() += 1;
        }
    }

    pub fn noted(&mut self, diagnostics: &[Diagnostic]) {
        for diagnostic in diagnostics {
            *self.0.entry(diagnostic.kind.name()).or_default() += 1;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteInfo {
    pub texture: [i32; 2],
    pub frames: i32,
    pub vertical: bool,
    pub masked: bool,
    pub animated: bool,
    pub kind: SpriteKind,
    pub anchor: [f32; 2],
    pub effect: Result<Effect, String>,
    pub hit: Hit,
    pub missing: Option<String>,
    pub refusals: Vec<(RefusalKind, String)>,
}

use content_reader_parser::crt::truncate_f32;

impl SpriteInfo {
    fn divisors(&self) -> [i32; 2] {
        if self.vertical {
            [1, self.frames]
        } else {
            [self.frames, 1]
        }
    }

    pub fn extent(&self) -> [i32; 2] {
        let divisors = self.divisors();
        [0, 1]
            .map(|axis| truncate_f32((1.0f32 / divisors[axis] as f32) * self.texture[axis] as f32))
    }

    pub fn frame_size(&self) -> [f32; 2] {
        let divisors = self.divisors();
        [0, 1].map(|axis| (1.0f32 / divisors[axis] as f32) * self.texture[axis] as f32)
    }

    fn frame_uv(&self) -> ([f32; 2], [f32; 2]) {
        let divisors = self.divisors();
        let span = [0, 1].map(|axis| {
            let size = self.texture[axis] as f32;
            (size * (1.0f32 / size)) * (1.0f32 / divisors[axis] as f32)
        });
        let step = if self.vertical {
            [0.0, span[1]]
        } else {
            [span[0], 0.0]
        };
        (span, step)
    }

    pub fn scaled_extent(&self, scale: f32) -> [i32; 2] {
        match self.kind {
            SpriteKind::Progress { size, .. } => size,
            SpriteKind::CorneredTile { size, .. } => size,
            SpriteKind::MaskedShield { .. } => self.texture,
            SpriteKind::PieChart { size } => [size.unwrap_or(0); 2],
            SpriteKind::CircularProgress { size, .. } => [size; 2],
            SpriteKind::LineChart { size } => size,
            _ => self
                .extent()
                .map(|value| truncate_f32(value as f32 * scale)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hit {
    #[default]
    Frame,
    Through,
    Alpha,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpriteKind {
    Plain,
    Progress {
        size: [i32; 2],
        steps: i32,
        colors: [[f32; 4]; 2],
        horizontal: bool,
        textured: bool,
    },
    CorneredTile {
        size: [i32; 2],
        border: [i32; 2],
        tiling_center: bool,
        looping: bool,
        rate: Option<f32>,
    },
    Tile { size: [i32; 2] },
    Resizeable,
    FrameAnimated {
        seconds_per_frame: f32,
        looping: bool,
        play_on_show: bool,
        pause: f32,
    },
    MaskedShield {
        mask: [i32; 2],
        small: bool,
    },
    PieChart { size: Option<i32> },
    LineChart { size: [i32; 2] },
    CircularProgress {
        size: i32,
        rotation: f32,
        amount: f32,
    },
    Text { textured: bool },
}

pub trait Resources {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        None
    }
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo>;

    fn sprite_info(&self, name: &[u8]) -> Option<std::borrow::Cow<'_, SpriteInfo>> {
        self.sprite(name).map(std::borrow::Cow::Owned)
    }
    fn portrait_texture(&self, _name: &[u8]) -> Option<Texture> {
        None
    }

    fn sprite_alpha(&self, _name: &[u8]) -> Option<AlphaMask> {
        None
    }
    fn unloaded_sprite(&self, _name: &[u8]) -> bool {
        false
    }
    fn font(&self, name: &[u8]) -> Option<&native_text::Font>;
    fn localisation(&self) -> &Localisation;
    fn text_colors(&self) -> &TextColors;
    fn text_effect(&self) -> Result<Effect, String>;
    fn effect(&self, file: &str) -> Result<Effect, String>;
    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String>;
    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String>;
    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
        Err(format!(
            "resolved_text_flag: atlas key {key:?} has no bound flag atlas"
        ))
    }
    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
        let _ = text;
        None
    }
    fn gui_type(&self, name: &[u8]) -> Option<&Element>;
    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        let _ = path;
        None
    }
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        let _ = sprite;
        None
    }
    fn session_text_sprite(&self, name: &[u8]) -> bool {
        let _ = name;
        false
    }
}

fn looked_up(
    resources: &dyn Resources,
    refused: &RefCell<Refusals>,
    path: &str,
    name: &[u8],
    creates_text_box: bool,
) -> Option<SpriteInfo> {
    looked_up_info(resources, refused, path, name, creates_text_box).map(Cow::into_owned)
}

fn looked_up_info<'a>(
    resources: &'a dyn Resources,
    refused: &RefCell<Refusals>,
    path: &str,
    name: &[u8],
    creates_text_box: bool,
) -> Option<Cow<'a, SpriteInfo>> {
    let info = resources.sprite_info(name);
    if info.is_none() && resources.unloaded_sprite(name) {
        let refusal = Refusal::new(
            path,
            RefusalKind::MissingResource,
            format!(
                "declared sprite {:?} has not been loaded (Assets::sprites)",
                lossy(name)
            ),
        );
        let mut refused = refused.borrow_mut();
        if !refused.contains(&refusal) {
            refused.push(refusal);
        }
        return None;
    }
    if info.is_none() && !creates_text_box && resources.session_text_sprite(name) {
        let refusal = Refusal::new(
            path,
            RefusalKind::MissingInput,
            format!(
                "looks up the sprite {:?}, which the content does not define and a textBoxType \
                 is named: constructing such a text box registers a text sprite type under that \
                 name for the session, so the lookup depends on earlier text box construction \
                 that the recording does not hold",
                lossy(name)
            ),
        );
        let mut refused = refused.borrow_mut();
        if !refused.contains(&refusal) {
            refused.push(refusal);
        }
        return None;
    }
    noted(refused, path, name, info)
}

pub(super) fn reader_refusal(refusal: &native::Refusal) -> Refusal {
    let kind = match refusal.cause {
        native::Cause::Unrecorded => RefusalKind::MissingInput,
        _ => RefusalKind::Reader,
    };
    Refusal::new(&refusal.path, kind, refusal.to_string())
}

pub(super) fn reader_refusals(element: &Element) -> Refusals {
    element
        .subtree_refusals()
        .into_iter()
        .map(reader_refusal)
        .collect()
}

fn localised(resources: &dyn Resources, key: &[u8]) -> Option<String> {
    Some(
        shown_chars(resources.localisation(), key)?
            .into_iter()
            .collect(),
    )
}

fn shown_chars(localisation: &Localisation, key: &[u8]) -> Option<Vec<char>> {
    native_text::decode(&localisation.localise_bytes(key, &[]).ok()?)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteSetter {
    #[default]
    Button,
    ButtonStandard,
    Icon,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    pub sprite: Option<Vec<u8>>,
    pub setter: SpriteSetter,
    pub frame: Option<i32>,
    pub text: Option<String>,
    pub resolved_flags: bool,
    shown: Option<lifecycle::Shown>,
    pub color: Option<[f32; 4]>,
    pub progress: Option<i32>,
    pub progress_float: Option<i32>,
    pub texture: Option<Texture>,
    pub stretch: Option<f32>,
    pub offset: Option<[i32; 2]>,
    pub rotation: Option<f32>,
    pub scale: Option<f32>,
    pub clip: Option<Clip>,
    pub caret: Option<usize>,
    opacity: Option<f32>,
    window_frame: Option<Frame>,
    layout_parent: Option<[i32; 2]>,
    pub position: Option<[i32; 2]>,
    drawn: Drawn,
    clock: Option<(Option<i32>, f32)>,
    pulse: Option<f32>,
    hang: Option<String>,
    pub shield: Option<i32>,
    pub checked: Option<bool>,
    pub enabled: Option<bool>,
    text_bar: Option<(widgets::PlainState, bool)>,
    text_scroll: Option<f32>,
    scroll: [Option<i32>; 2],
    grid: Option<GridBounds>,
    bars: [Option<widgets::ContainerBar>; 2],
    animation_time: Option<f32>,
    effect_since: Option<f32>,
    effect_state: Option<u32>,
    triggers: Option<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextureFile {
    Loaded(Texture),
    Failed(String),
    Refused(RefusalKind, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texture {
    pub texture: usize,
    pub uv: [f32; 4],
    pub size: [i32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Drawn {
    #[default]
    Written,
    Kept(Option<(std::sync::Arc<[u8]>, SpriteSetter)>),
}

fn made<'a>(
    bound: Option<&'a Bound>,
    authored: &[u8],
    resources: &dyn Resources,
) -> Option<(&'a [u8], SpriteSetter)> {
    let bound = bound?;
    match &bound.drawn {
        Drawn::Kept(kept) => kept
            .as_ref()
            .map(|(sprite, setter)| (sprite.as_ref(), *setter)),
        Drawn::Written => {
            let name = bound.sprite.as_deref()?;
            setter_replaces(authored, name, resources).then_some((name, bound.setter))
        }
    }
}

pub(super) fn setter_replaces(current: &[u8], name: &[u8], resources: &dyn Resources) -> bool {
    name != current && sprite_resolves(name, resources)
}

pub fn sprite_resolves(name: &[u8], resources: &dyn Resources) -> bool {
    resources.sprite_info(name).is_some() || resources.session_text_sprite(name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Role {
    Part(u8),
    Scrollbar,
    Bar(bool),
    Background,
}

pub(super) type Via = (*const Element, Role);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Site {
    element: *const Element,
    via: Option<Via>,
}

impl Site {
    pub(super) fn new(element: &Element, via: Option<Via>) -> Self {
        Self {
            element: std::ptr::from_ref::<Element>(element),
            via,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Bindings {
    bound: HashMap<Site, Bound>,
    pub regions: bool,
    pub time: f32,
    pub pointer: Option<Pointer>,
    pub viewport: Option<[i32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pointer {
    pub at: [f32; 2],
    pub pressed: Option<String>,
    pub over: Option<String>,
    pub clip_at: Option<[f32; 2]>,
    pub drawn: Option<String>,
    pub drawn_boxes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pointed {
    pub path: String,
    pub down: bool,
}

impl Bindings {
    pub fn bind(&mut self, element: &Element) -> &mut Bound {
        self.bind_at(Site::new(element, None))
    }

    pub(super) fn bind_at(&mut self, site: Site) -> &mut Bound {
        self.bound.entry(site).or_default()
    }

    fn at(&self, site: Site) -> Option<&Bound> {
        self.bound.get(&site)
    }
}

pub type Rect = [f32; 4];

pub type Clip = [i32; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnGlyph {
    pub ch: char,
    pub at: [f32; 2],
    pub color: [f32; 4],
    pub texture: usize,
    pub size: [i32; 2],
    pub uv: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    pub compiled: Option<super::native_shader::draw::Effect>,
    pub fixture: Option<(FixtureTechnique, Blend, [Option<Sampler>; 6])>,
    pub since: f32,
    pub animation: f32,
    pub time: f32,
    pub clocked: bool,
}

impl Shading {
    pub fn settles(&self) -> Settles {
        if let Some(compiled) = &self.compiled {
            return super::native_effects::compiled_settles(&compiled.asset, &compiled.technique);
        }
        match self.fixture {
            Some((fixture, ..)) => Settles::Program(fixture.settles),
            None => Settles::Previous,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub path: String,
    pub child: Option<usize>,
    pub background_index: Option<usize>,
    pub kind: PartKind,
    pub clip: Option<Clip>,
    pub color: [f32; 4],
    pub shading: Shading,
    pub native: Option<Box<super::native_shader::draw::Part>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub rect: Rect,
    pub rotation: f32,
    pub pivot: [f32; 2],
    pub local: Rect,
}

impl Geometry {
    fn turn(&self, point: [f32; 2]) -> [f32; 2] {
        if self.rotation == 0.0 {
            return point;
        }
        let (sin, cos) = self.rotation.sin_cos();
        let [dx, dy] = [point[0] - self.pivot[0], point[1] - self.pivot[1]];
        [
            self.pivot[0] + dx * cos + dy * sin,
            self.pivot[1] + dy * cos - dx * sin,
        ]
    }

    pub fn corners(&self) -> [[f32; 2]; 4] {
        let [x, y, w, h] = self.rect;
        [[x, y], [x + w, y], [x + w, y + h], [x, y + h]].map(|point| self.turn(point))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PartKind {
    Sprite {
        sprite: Vec<u8>,
        geometry: Geometry,
        uv: [f32; 4],
        offsets: [f32; 4],
        masked: bool,
    },
    Tiles {
        sprite: Vec<u8>,
        quads: Vec<(Rect, [f32; 4])>,
        native_origin: [f32; 2],
        cornered: bool,
    },
    MaskedFlag {
        sprite: Vec<u8>,
        flag: usize,
        flag_coords: [f32; 4],
        rect: Rect,
        native_origin: [f32; 2],
    },
    Cell {
        texture: usize,
        masking: Option<Vec<u8>>,
        uv: [f32; 4],
        geometry: Geometry,
    },
    Progress {
        sprite: Vec<u8>,
        geometry: Geometry,
        uv: [[f32; 2]; 4],
        state: f32,
        colors: [[f32; 4]; 2],
        textured: bool,
    },
    Inline {
        source: InlineSource,
        rect: Rect,
        uv: [f32; 4],
        modulate: [f32; 4],
        projection: [f32; 2],
        font_viewport: Option<[i32; 2]>,
    },
    Fan {
        sprite: Vec<u8>,
        centre: [f32; 2],
        radius: [f32; 2],
        rotation: f32,
        weights: [i32; 2],
    },
    Animated(Box<Animated>),
    TextSprite {
        sprite: Vec<u8>,
        bake: Box<text_sprites::TextBake>,
        geometry: Geometry,
        uv: [f32; 4],
        offsets: [f32; 4],
    },
    Text {
        font: Vec<u8>,
        glyphs: Vec<DrawnGlyph>,
        scale: f32,
        modulate: [f32; 4],
        border_color: u32,
        projection: [f32; 2],
        font_viewport: Option<[i32; 2]>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InlineSource {
    Sprite(Vec<u8>),
    Texture(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: [i32; 2],
    pub parts: Vec<Part>,
    pub holders: Vec<Holder>,
    pub regions: Vec<Region>,
    pub diagnostics: Vec<Diagnostic>,
    pub pointed: Option<Pointed>,
    pub hovered: Option<String>,
    pub target: Option<Target>,
    pub boxes: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl Layout {
    pub fn as_drawn(mut self) -> Self {
        for part in &mut self.parts {
            if matches!(part.shading.settles(), Settles::Program(_)) && !part.shading.changing() {
                (part.shading.since, part.shading.animation, part.shading.time) = (0.0, 0.0, 0.0);
            }
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    pub check_box: bool,
    pub checked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub path: String,
    pub owner: Option<String>,
    pub background_index: Option<usize>,
    pub scroll: Option<(bool, i32)>,
    pub slide: Option<Slide>,
    pub rect: Rect,
    pub clip: Option<Clip>,
    pub hit: Option<HitGeometry>,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slide {
    pub vertical: bool,
    pub travel: i32,
    pub thumb: i32,
}

impl Slide {
    pub fn dragged(self, moved: i32, [low, high]: [i32; 2]) -> i32 {
        let travel = self.travel.max(0);
        if travel == 0 {
            return low;
        }
        let offset = self.thumb.wrapping_add(moved).clamp(0, travel);
        let low_f = low as f32;
        truncate_half_away((high as f32 - low_f) * (offset as f32 / travel as f32) + low_f)
    }
}

pub(crate) fn truncate_half_away(value: f32) -> i32 {
    content_reader_parser::crt::truncate_f32(if value >= 0.0 { value + 0.5 } else { value - 0.5 })
}

impl Layout {
    pub fn composed_parts<'a>(&'a self, items: &'a [Vec<Part>]) -> impl Iterator<Item = &'a Part> {
        let (mut part, mut holder, mut item) = (0, 0, 0);
        std::iter::from_fn(move || {
            loop {
                if holder < self.holders.len()
                    && (part == self.parts.len() || self.holders[holder].before <= part)
                {
                    if let Some(next) = items.get(holder).and_then(|items| items.get(item)) {
                        item += 1;
                        return Some(next);
                    }
                    holder += 1;
                    item = 0;
                } else if let Some(next) = self.parts.get(part) {
                    part += 1;
                    return Some(next);
                } else {
                    return None;
                }
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub path: String,
    pub child: Option<usize>,
    pub content: [i32; 2],
    pub origin: [f32; 2],
    pub scale: f32,
    pub hit_scales: Option<Vec<(f32, [i32; 2])>>,
    pub logical: [i32; 2],
    pub clip: Option<Clip>,
    pub opacity: f32,
    pub before: usize,
    pub position: Option<[i32; 2]>,
}

pub fn normalized_progress_value(fraction: f32, steps: i32) -> i32 {
    let value = (f64::from(fraction.clamp(0.0, 1.0)) * f64::from(steps)) as f32;
    truncate_f32(if value < 0.0 {
        value - 0.5
    } else {
        value + 0.5
    })
}

fn round(value: f32) -> i32 {
    if value < 0.0 {
        (value - 0.5).ceil() as i32
    } else {
        (value + 0.5).floor() as i32
    }
}

fn percent(coordinate: Coordinate, parent: i32) -> i32 {
    if coordinate.percent {
        round(coordinate.value.wrapping_mul(parent) as f32 * 0.01)
    } else {
        coordinate.value
    }
}

fn no_parent(what: impl std::fmt::Display) -> String {
    format!(
        "{what} is a percentage or measures from the parent's far edge, and the parent's size \
         is not given: a root with no parent object is sized in the GUI viewport, the device \
         size over the interface scale, which the caller passes"
    )
}

pub fn evaluate_position(
    position: &native::Position,
    parent: Option<[i32; 2]>,
    what: impl std::fmt::Display,
) -> Result<[i32; 2], String> {
    let mut out = [0; 2];
    for (axis, coordinate) in [position.x, position.y].into_iter().enumerate() {
        let needs = |coordinate: Coordinate| coordinate.percent && !coordinate.auto;
        if parent.is_none()
            && (coordinate.percent || needs(position.min[axis]) || needs(position.max[axis]))
        {
            return Err(no_parent(what));
        }
        let extent = parent.map_or(0, |parent| parent[axis]);
        let mut value = percent(coordinate, extent);
        if !position.min[axis].auto {
            value = value.max(percent(position.min[axis], extent));
        }
        if !position.max[axis].auto {
            value = value.min(percent(position.max[axis], extent));
        }
        out[axis] = value;
    }
    Ok(out)
}

pub fn evaluate_size(
    block: &native::Size,
    parent: Option<[i32; 2]>,
    position: [i32; 2],
    what: impl std::fmt::Display,
) -> Result<[i32; 2], String> {
    let coordinates = || {
        block.size.iter().chain(
            block
                .min
                .iter()
                .chain(&block.max)
                .filter(|coordinate| !coordinate.auto),
        )
    };
    if parent.is_none()
        && coordinates().any(|coordinate| coordinate.percent || coordinate.value < 0)
    {
        return Err(no_parent(what));
    }
    let parent = parent.unwrap_or([0; 2]);
    let resolve = |coordinate: Coordinate, axis: usize| -> i32 {
        let mut value = percent(coordinate, parent[axis]);
        if value < 0 {
            value = value
                .wrapping_sub(position[axis])
                .wrapping_add(parent[axis]);
        }
        if coordinate.from_position {
            value.wrapping_sub(position[axis])
        } else {
            value
        }
    };
    let bound = |pair: [Coordinate; 2]| -> [Option<i32>; 2] {
        [0, 1].map(|axis| (!pair[axis].auto).then(|| resolve(pair[axis], axis)))
    };
    let whole = [resolve(block.size[0], 0), resolve(block.size[1], 1)];
    let (min, max) = (bound(block.min), bound(block.max));
    let mut size = whole;
    for axis in 0..2 {
        if let Some(min) = min[axis]
            && size[axis] < min
        {
            size[axis] = min;
        }
        if let Some(max) = max[axis]
            && max < size[axis]
        {
            size[axis] = max;
        }
    }
    if block.preserve_aspect_ratio && whole.iter().chain(&size).all(|&value| value > 0) {
        let ratio = whole[0] as f32 / whole[1] as f32;
        let [width, height] = size.map(|value| value as f32);
        let lower = |value: i32, bound: Option<i32>| bound.map_or(value, |min| value.max(min));
        let upper = |value: i32, bound: Option<i32>| bound.map_or(value, |max| value.min(max));
        if width / height < ratio {
            if min[1] != Some(size[1]) {
                size[1] = lower(round(width / ratio), min[1]);
            } else {
                size[0] = upper(round(height * ratio), max[0]);
            }
        } else if width / height > ratio {
            if min[0] != Some(size[0]) {
                size[0] = lower(round(height * ratio), min[0]);
            } else {
                size[1] = upper(round(width / ratio), max[1]);
            }
        }
    }
    Ok(size)
}

pub fn compute_anchor(
    orientation: Orientation,
    origo: Orientation,
    parent: [i32; 2],
    own: [i32; 2],
) -> [i32; 2] {
    let [pw, ph] = parent;
    let [w, h] = own;
    let from_parent = match orientation {
        Orientation::UpperLeft => [0, 0],
        Orientation::UpperRight => [pw, 0],
        Orientation::LowerLeft => [0, ph],
        Orientation::LowerRight => [pw, ph],
        Orientation::Center => [pw / 2, ph / 2],
        Orientation::CenterLeft => [0, ph / 2],
        Orientation::CenterRight => [pw, ph / 2],
        Orientation::CenterUp => [pw / 2, 0],
        Orientation::CenterDown => [pw / 2, ph],
    };
    let from_own = match origo {
        Orientation::UpperLeft => [0, 0],
        Orientation::UpperRight => [-w, 0],
        Orientation::LowerLeft => [0, -h],
        Orientation::LowerRight => [-w, -h],
        Orientation::Center => [-w / 2, -h / 2],
        Orientation::CenterLeft => [0, -h / 2],
        Orientation::CenterRight => [-w, -h / 2],
        Orientation::CenterUp => [-w / 2, 0],
        Orientation::CenterDown => [-w / 2, -h],
    };
    [from_parent[0] + from_own[0], from_parent[1] + from_own[1]]
}

pub fn control_anchor(orientation: Orientation, parent: [i32; 2]) -> [i32; 2] {
    let [w, h] = parent;
    match orientation {
        Orientation::UpperRight => [w, 0],
        Orientation::LowerLeft => [0, h],
        Orientation::LowerRight => [w, h],
        Orientation::Center => [w / 2, h / 2],
        Orientation::CenterUp => [w / 2, 0],
        Orientation::CenterDown => [w / 2, h],
        Orientation::UpperLeft | Orientation::CenterLeft | Orientation::CenterRight => [0, 0],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub at: [i32; 2],
    pub size: [i32; 2],
    pub content_at: [i32; 2],
    pub content_size: [i32; 2],
}

fn margin_sides(margin: &native::Margin, size: [i32; 2]) -> [i32; 4] {
    [
        percent(margin.left, size[0]),
        percent(margin.top, size[1]),
        percent(margin.right, size[0]),
        percent(margin.bottom, size[1]),
    ]
}

pub fn container_frame(window: &Element, parent: Option<[i32; 2]>) -> Result<Frame, String> {
    let container = window
        .container()
        .ok_or_else(|| format!("{} is not a containerWindowType", window.path))?;
    let show = evaluate_position(
        &container.show_position,
        parent,
        format_args!("{}.show_position", window.path),
    )?;
    let size = evaluate_size(
        &container.size,
        parent,
        show,
        format_args!("{}.size", window.path),
    )?;
    frame_from_extent(window, container, parent, size)
}

fn frame_from_extent(
    window: &Element,
    container: &native::Container,
    parent: Option<[i32; 2]>,
    size: [i32; 2],
) -> Result<Frame, String> {
    let at = match parent {
        Some(parent) => {
            let position = evaluate_position(
                &container.position,
                Some(parent),
                format_args!("{}.position", window.path),
            )?;
            let anchor = compute_anchor(container.orientation, container.origo, parent, size);
            [position[0] + anchor[0], position[1] + anchor[1]]
        }
        None => [0, 0],
    };
    let [left, top, right, bottom] = margin_sides(&container.margin, size);
    Ok(Frame {
        at,
        size,
        content_at: [left, top],
        content_size: [size[0] - left - right, size[1] - top - bottom],
    })
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootPlacement {
    Constructed,
    Set([i32; 2]),
    Anchored([i32; 2]),
}

#[cfg(any(test, feature = "test-support"))]
pub fn root_origin(
    window: &Element,
    parent: [i32; 2],
    placement: RootPlacement,
) -> Result<[i32; 2], String> {
    let container = window
        .container()
        .ok_or_else(|| format!("{} is not a containerWindowType", window.path))?;
    let frame = container_frame(window, Some(parent))?;
    let anchor = |point: [i32; 2]| {
        let anchor = compute_anchor(container.orientation, container.origo, parent, frame.size);
        [point[0] + anchor[0], point[1] + anchor[1]]
    };
    Ok(match placement {
        RootPlacement::Constructed => frame.at,
        RootPlacement::Set(point) => point,
        RootPlacement::Anchored(point) => anchor(point),
    })
}

fn cornered_quads(
    size: [i32; 2],
    border: [i32; 2],
    texture: [i32; 2],
    frames: i32,
    frame: i32,
    tiling: bool,
) -> Vec<(Rect, [f32; 4])> {
    let [w, h] = size;
    let [bx, by] = border;
    let [tex_w, tex_h] = texture.map(|value| value as f32);
    let fi = if frame == 0 || frame.wrapping_sub(1) as u32 >= frames as u32 {
        0
    } else {
        frame.wrapping_sub(1) as u32
    };
    let fw = 1.0f32 / frames as u32 as f32;
    let u0 = fi as f32 * fw;
    let u1 = u0 + fw;
    let bu = bx as f32 / tex_w;
    let bv = by as f32 / tex_h;
    let eu = if bx != 0 {
        (bx + 1) as f32 / tex_w
    } else {
        0.0
    };
    let ev = if by != 0 {
        (by + 1) as f32 / tex_h
    } else {
        0.0
    };
    let tw = texture[0] - 2 * bx;
    let th = texture[1] - 2 * by;
    let nx = if tw != 0 { (w - 2 * bx) / tw } else { 0 };
    let ny = if th != 0 { (h - 2 * by) / th } else { 0 };
    let mut quads = Vec::new();
    let mut quad = |x0: i32, x1: i32, y0: i32, y1: i32, uv: [f32; 4]| {
        quads.push((
            [x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32],
            uv,
        ));
    };
    quad(0, bx, 0, by, [u0, 0.0, u0 + bu, bv]);
    quad(w - bx, w, 0, by, [u1 - bu, 0.0, u1, bv]);
    quad(w - bx, w, h - by, h, [u1 - bu, 1.0 - bv, u1, 1.0]);
    quad(0, bx, h - by, h, [u0, 1.0 - bv, u0 + bu, 1.0]);
    let mut x = bx;
    for _ in 0..nx.max(0) {
        quad(x, x + tw, 0, by, [u0 + bu, 0.0, u1 - eu, bv]);
        quad(x, x + tw, h - by, h, [u0 + bu, 1.0 - bv, u1 - eu, 1.0]);
        x += tw;
    }
    let rest = (w - bx) - x;
    if rest != 0 {
        let end = (rest + bx) as f32 / tex_w;
        quad(x, w - bx, 0, by, [u0 + bu, 0.0, end, bv]);
        quad(x, w - bx, h - by, h, [u0 + bu, 1.0 - bv, end, 1.0]);
    }
    let mut y = by;
    for _ in 0..ny.max(0) {
        quad(0, bx, y, y + th, [u0, bv, u0 + bu, 1.0 - ev]);
        quad(w - bx, w, y, y + th, [u1 - bu, bv, u1, 1.0 - ev]);
        y += th;
    }
    let rest = (h - by) - y;
    if rest != 0 {
        let end = (rest + by) as f32 / tex_h;
        quad(0, bx, y, h - by, [u0, bv, u0 + bu, end]);
        quad(w - bx, w, y, h - by, [u1 - bu, bv, u1, end]);
    }
    if !tiling {
        if w - 2 * bx > 0 && h - 2 * by > 0 {
            quad(bx, w - bx, by, h - by, [u0 + bu, bv, u1 - bu, 1.0 - bv]);
        }
    } else {
        let mut rows = Vec::new();
        let mut y = by;
        for _ in 0..ny.max(0) {
            rows.push((y, y + th, 1.0 - ev));
            y += th;
        }
        if (h - by) - y != 0 {
            rows.push((y, h - by, ((h - by) - y + by) as f32 / tex_h));
        }
        for (y0, y1, v1) in rows {
            let mut x = bx;
            for _ in 0..nx.max(0) {
                quad(x, x + tw, y0, y1, [u0 + bu, bv, u1 - eu, v1]);
                x += tw;
            }
            if (w - bx) - x != 0 {
                let end = ((w - bx) - x + bx) as f32 / tex_w;
                quad(x, w - bx, y0, y1, [u0 + bu, bv, end, v1]);
            }
        }
    }
    quads
}

pub(super) fn drawn_sprite<T>(
    name: &[u8],
    mut lookup: impl FnMut(&[u8]) -> Option<T>,
) -> Option<(Vec<u8>, T)> {
    [name, FALLBACK_SPRITE]
        .into_iter()
        .find_map(|name| Some((name.to_vec(), lookup(name)?)))
}

pub(super) fn text_only(button: &native::Button, sprite: &[u8], text: Option<&str>) -> bool {
    sprite.is_empty() && !button.font.is_empty() && text != Some("")
}

pub(super) fn own_sprite(element: &Element) -> Option<(&[u8], bool)> {
    match &element.fields {
        Fields::Icon(icon) => Some((&icon.sprite, true)),
        Fields::Button(button) if element.kind == Kind::Checkbox => {
            Some((&button.sprite, button.font.is_empty()))
        }
        Fields::Button(button) => Some((&button.sprite, true)),
        Fields::ListBox(list) => {
            let name = content_reader_parser::parser::c_string(&list.background);
            (!name.is_empty()).then_some((name, true))
        }
        Fields::EditBox(edit) if edit.flags() & 0x2 == 0 => Some((b"", false)),
        _ => None,
    }
}

pub(super) const EDIT_CARET: &[u8] = b"cursor";

fn visible(element: &Element, bound: Option<&Bound>) -> bool {
    bound
        .and_then(|bound| bound.shown)
        .map_or(!element.hide, |shown| shown.visible)
}

fn window_draws(window: &Element, bound: Option<&Bound>) -> Result<bool, Refusal> {
    let Some(container) = window.container() else {
        return Err(Refusal::new(
            &window.path,
            RefusalKind::Reader,
            "is not a containerWindowType",
        ));
    };
    let path = &window.path;
    if container.fade != Animation::None {
        if container.fade_time <= 0 {
            return Ok(false);
        }
        if bound.and_then(|bound| bound.opacity).is_none() {
            return Err(Refusal::new(
                path,
                RefusalKind::MissingInput,
                "fades (fade_type) on its instance's clock: lay it out through its lifecycle \
                 Instance",
            ));
        }
    }
    if (container.show_animation != Animation::None || container.hide_animation != Animation::None)
        && bound.and_then(|bound| bound.position).is_none()
    {
        return Err(Refusal::new(
            path,
            RefusalKind::MissingInput,
            "moves when shown or hidden (show_animation_type, hide_animation_type) on its \
             instance's clock: lay it out through its lifecycle Instance",
        ));
    }
    Ok(true)
}

fn intersect(a: Option<Clip>, b: Clip) -> Option<Clip> {
    let Some(a) = a else {
        return Some(b);
    };
    Some([
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ])
}

struct Walk<'a> {
    resources: &'a dyn Resources,
    bindings: &'a Bindings,
    parts: Vec<Part>,
    holders: Vec<Holder>,
    regions: Vec<Region>,
    diagnostics: Vec<Diagnostic>,
    refusals: Refusals,
    top: Option<usize>,
    background_index: Option<usize>,
    opacity: f32,
    hits: Vec<Hittable>,
    hovered: Option<String>,
    boxes: Vec<String>,
    pointed: Option<Pointed>,
    target: Option<Target>,
    text_effect: Option<Result<Effect, String>>,
    sized: Option<[i32; 2]>,
    via: Option<Via>,
    region_owner: Option<String>,
    refused: RefCell<Refusals>,
}

struct Hittable {
    path: String,
    part: usize,
    hit: Hit,
    check_box: bool,
    checked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Up,
    Down(f32),
    Over(f32),
    Disable,
}

impl State {
    fn number(self) -> u32 {
        match self {
            Self::Up => 1,
            Self::Down(_) => 2,
            Self::Over(_) => 3,
            Self::Disable => 4,
        }
    }

    fn technique(self) -> &'static str {
        match self {
            Self::Up => "Up",
            Self::Down(_) => "Down",
            Self::Over(_) => "Over",
            Self::Disable => "Disable",
        }
    }

    fn since(self) -> f32 {
        match self {
            Self::Down(since) | Self::Over(since) => since,
            Self::Up | Self::Disable => 0.0,
        }
    }
}

fn shading(effect: &Effect, technique: &str, since: f32) -> Shading {
    Shading {
        compiled: effect
            .compiled
            .as_ref()
            .map(|asset| super::native_shader::draw::Effect {
                asset: asset.clone(),
                technique: technique.to_owned(),
            }),
        fixture: match effect.technique(technique) {
            Technique::Run { fixture, blend } => Some((fixture, blend, effect.samplers)),
            Technique::Keep => None,
        },
        since,
        animation: 0.0,
        time: 0.0,
        clocked: true,
    }
}

fn layout(
    window: &Element,
    parent: Option<[i32; 2]>,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<Option<Layout>, Refusals> {
    let reader = reader_refusals(window);
    if !reader.is_empty() {
        return Err(reader);
    }
    let frame = bindings
        .at(Site::new(window, None))
        .and_then(|bound| bound.window_frame)
        .map_or_else(|| container_frame(window, parent), Ok)
        .map_err(|why| vec![Refusal::new(&window.path, RefusalKind::NoParent, why)])?;
    let walk = |hovered: Option<String>, boxes: Vec<String>| -> Result<Option<Walk>, Refusals> {
        let mut walk = Walk {
            hovered,
            boxes,
            ..Walk::new(resources, bindings)
        };
        let drawn = match walk.window(window, [0, 0], frame, None) {
            Ok(drawn) => drawn,
            Err(refusal) => {
                walk.refusals.push(refusal);
                false
            }
        };
        let refused = walk.refused.take();
        walk.refusals.extend(refused);
        if !walk.refusals.is_empty() {
            return Err(walk.refusals);
        }
        Ok(drawn.then_some(walk))
    };
    let first = bindings.pointer.as_ref().map_or((None, Vec::new()), |pointer| {
        (pointer.drawn.clone(), pointer.drawn_boxes.clone())
    });
    let Some(mut drawn) = walk(first.0.clone(), first.1.clone())? else {
        return Ok(None);
    };
    if let Some(pointer) = &bindings.pointer {
        let (hovered, boxes, target) = match &pointer.over {
            Some(over) => (
                Some(over.clone()),
                Vec::new(),
                Some(Target {
                    path: over.clone(),
                    check_box: false,
                    checked: false,
                }),
            ),
            None => pointer_targets(&drawn.parts, &drawn.hits, &drawn.regions, pointer)?,
        };
        let hovered = match &pointer.pressed {
            Some(pressed) => hovered.filter(|hovered| hovered == pressed),
            None => hovered,
        };
        if (&hovered, &boxes) != (&first.0, &first.1) || pointer.pressed.is_some() {
            drawn = walk(hovered, boxes)?.expect("drawn in the first pass");
        }
        drawn.target = target;
    }
    Ok(Some(drawn.finish(frame.size)))
}

#[cfg(test)]
pub(super) fn partial_layout_for_tests(
    window: &Element,
    parent: [i32; 2],
    resources: &dyn Resources,
) -> (Layout, Refusals) {
    let bindings = Bindings {
        regions: true,
        ..Bindings::default()
    };
    let frame = container_frame(window, Some(parent)).unwrap();
    let mut walk = Walk::new(resources, &bindings);
    let _ = walk.window(window, [0, 0], frame, None);
    let refusals = std::mem::take(&mut walk.refusals);
    (walk.finish(frame.size), refusals)
}

pub fn pointed_button(
    window: &Element,
    element: &Element,
    resources: &dyn Resources,
    mut bindings: Bindings,
    down: Option<bool>,
    since: f32,
) -> Result<Layout, Refusals> {
    let refuse = |why: &str| vec![Refusal::new(&element.path, RefusalKind::MissingInput, why)];
    let button = element.button().ok_or_else(|| refuse("is not a button"))?;
    let frame = container_frame(window, bindings.viewport).map_err(|why| refuse(&why))?;
    let bound = bindings.bind(element);
    bound.effect_state = Some(match down {
        None => 1,
        Some(true) => 2,
        Some(false) => 3,
    });
    bound.effect_since = Some(since);
    bindings.pointer = Some(Pointer {
        at: [f32::NAN; 2],
        pressed: (down == Some(true)).then(|| element.path.clone()),
        over: None,
        clip_at: None,
        drawn: None,
        drawn_boxes: Vec::new(),
    });
    let mut walk = Walk {
        hovered: (down == Some(false)).then(|| element.path.clone()),
        ..Walk::new(resources, &bindings)
    };
    let Fields::Container(container) = &window.fields else {
        return Err(refuse("is not in a container window"));
    };
    if !window.children.iter().any(|child| std::ptr::eq(child, element)) {
        return Err(refuse("is not a child of its window"));
    }
    if container.scale != 1.0 {
        return Err(refuse("is in a scaled window"));
    }
    let content = frame.content_at;
    let scroll = walk.content_scroll(window, container);
    let offset = walk.scroll_offset(window, &scroll);
    let moved = [
        content[0].wrapping_add(offset[0]),
        content[1].wrapping_add(offset[1]),
    ];
    let clips = container
        .clipping
        .unwrap_or(!container.backgrounds.is_empty());
    let clip = clips.then(|| {
        [
            content[0],
            content[1],
            content[0] + frame.content_size[0],
            content[1] + frame.content_size[1],
        ]
    });
    if let Err(refusal) = walk.button(element, button, moved, frame.content_size, clip) {
        walk.refusals.push(refusal);
    }
    let refused = walk.refused.take();
    walk.refusals.extend(refused);
    if !walk.refusals.is_empty() {
        return Err(walk.refusals);
    }
    Ok(walk.finish(frame.size))
}

impl Walk<'_> {
    fn finish(mut self, size: [i32; 2]) -> Layout {
        let time = self.bindings.time;
        for part in self.parts.iter_mut().filter(|part| part.shading.clocked) {
            part.shading.time = time;
        }
        Layout {
            size,
            parts: self.parts,
            holders: self.holders,
            regions: self.regions,
            diagnostics: self.diagnostics,
            pointed: self.pointed,
            hovered: self.hovered,
            target: self.target,
            boxes: self.boxes,
        }
    }
}

fn pointer_targets(
    parts: &[Part],
    hits: &[Hittable],
    regions: &[Region],
    pointer: &Pointer,
) -> Result<(Option<String>, Vec<String>, Option<Target>), Refusals> {
    let mut hovered = None;
    let mut target = None;
    let mut boxes = Vec::new();
    let clip_at = pointer.clip_at.unwrap_or(pointer.at);
    for hit in hits {
        let part = &parts[hit.part];
        let Some(rect) = part_rect(&part.kind) else {
            continue;
        };
        let inside = |rect: Rect, at: [f32; 2]| {
            (rect[0]..rect[0] + rect[2]).contains(&at[0])
                && (rect[1]..rect[1] + rect[3]).contains(&at[1])
        };
        let clipped = part.clip.is_some_and(|clip| {
            !inside(
                [
                    clip[0] as f32,
                    clip[1] as f32,
                    (clip[2] - clip[0]) as f32,
                    (clip[3] - clip[1]) as f32,
                ],
                clip_at,
            )
        });
        if !inside(rect, pointer.at) || clipped {
            continue;
        }
        match hit.hit {
            Hit::Through => continue,
            Hit::Alpha => {
                let geometry = regions
                    .iter()
                    .rev()
                    .find(|region| region.path == hit.path)
                    .and_then(|region| region.hit.as_ref());
                match geometry {
                    Some(geometry) if !geometry.contains_unclipped(pointer.at) => continue,
                    Some(_) => {}
                    None => {
                        return Err(vec![Refusal::new(
                            &hit.path,
                            RefusalKind::MissingResource,
                            "is under the pointer and tests its frame's alpha, which a layout \
                             without hit geometry does not hold",
                        )]);
                    }
                }
            }
            Hit::Frame => {}
        }
        target = Some(Target {
            path: hit.path.clone(),
            check_box: hit.check_box,
            checked: hit.checked,
        });
        if hit.check_box {
            boxes.push(hit.path.clone());
        } else {
            hovered = Some(hit.path.clone());
        }
    }
    Ok((hovered, boxes, target))
}

pub(crate) fn part_rect(kind: &PartKind) -> Option<Rect> {
    match kind {
        PartKind::Sprite { geometry, .. }
        | PartKind::Cell { geometry, .. }
        | PartKind::Progress { geometry, .. }
        | PartKind::TextSprite { geometry, .. } => Some(geometry.rect),
        PartKind::Animated(animated) => Some(animated.geometry.rect),
        PartKind::Tiles { quads, .. } => quads.first().map(|(first, _)| {
            quads.iter().fold(*first, |[x, y, w, h], (rect, _)| {
                let x0 = x.min(rect[0]);
                let y0 = y.min(rect[1]);
                let x1 = (x + w).max(rect[0] + rect[2]);
                let y1 = (y + h).max(rect[1] + rect[3]);
                [x0, y0, x1 - x0, y1 - y0]
            })
        }),
        PartKind::MaskedFlag { rect, .. } => Some(*rect),
        PartKind::Text { .. } | PartKind::Inline { .. } | PartKind::Fan { .. } => None,
    }
}

impl<'a> Walk<'a> {
    fn bound(&self, element: &Element) -> Option<&'a Bound> {
        self.bindings.at(Site::new(element, self.via))
    }
}

impl<'a> Walk<'a> {
    fn effect_clock(&self, bound: Option<&Bound>, state: State) -> f32 {
        let unticked = bound.is_some_and(|b| b.effect_state.is_some_and(|t| t != state.number()));
        let since = bound
            .filter(|_| !unticked)
            .and_then(|b| b.effect_since)
            .or(match state {
                State::Down(since) | State::Over(since) => Some(since),
                _ => unticked.then_some(0.0),
            });
        since.map_or(0.0, |since| self.bindings.time - since)
    }

    fn pointer_since(&self, bound: Option<&Bound>, state: u32) -> f32 {
        bound
            .filter(|bound| bound.effect_state == Some(state))
            .and_then(|bound| bound.effect_since)
            .unwrap_or(0.0)
    }

    fn built<T>(&mut self, owner: &Element, role: Role, draw: impl FnOnce(&mut Self) -> T) -> T {
        let outer = self.via.replace((std::ptr::from_ref::<Element>(owner), role));
        let outer_owner = self.region_owner.clone();
        if outer_owner.is_none() {
            self.region_owner = Some(owner.path.clone());
        }
        let drawn = draw(self);
        self.via = outer;
        self.region_owner = outer_owner;
        drawn
    }

    fn offset(&self, element: &Element) -> [i32; 2] {
        self.bound(element)
            .and_then(|bound| bound.offset)
            .unwrap_or([0; 2])
    }

    fn shown(&self, element: &Element) -> bool {
        visible(element, self.bound(element))
    }

    fn shown_text(&mut self, element: &Element, key: &[u8]) -> Option<String> {
        if let Some(text) = self.bound(element).and_then(|bound| bound.text.clone()) {
            return Some(text);
        }
        self.authored_text(element, key)
    }

    fn authored_text(&mut self, element: &Element, key: &[u8]) -> Option<String> {
        let text = localised(self.resources, key);
        if text.is_none() {
            self.diagnostics.push(Diagnostic {
                path: element.path.clone(),
                kind: DiagnosticKind::UndecodableText,
                detail: format!(
                    "shows the localisation of {:?}, whose bytes are not UTF-8: the game draws \
                     nothing of it",
                    lossy(key)
                ),
            });
        }
        text
    }

    fn text_effect(&mut self) -> Result<Effect, String> {
        let resources = self.resources;
        self.text_effect
            .get_or_insert_with(|| resources.text_effect())
            .clone()
    }

    fn window(
        &mut self,
        window: &Element,
        origin: [i32; 2],
        frame: Frame,
        clip: Option<Clip>,
    ) -> Result<bool, Refusal> {
        let bound = self.bound(window);
        let own_clip = bound.and_then(|bound| bound.clip);
        if !self.shown(window) || !window_draws(window, bound)? {
            return Ok(false);
        }
        let container = window.container().expect("checked by window_draws");
        if self.bindings.regions {
            self.regions.push(Region {
                path: window.path.clone(),
                owner: self.region_owner.clone(),
                background_index: None,
                scroll: None,
                slide: None,
                rect: [
                    origin[0] as f32,
                    origin[1] as f32,
                    frame.size[0] as f32,
                    frame.size[1] as f32,
                ],
                clip,
                hit: self
                    .bindings
                    .viewport
                    .map(|viewport| HitGeometry::rectangle(origin, frame.size, viewport, clip)),
                disabled: false,
            });
        }
        let outer_clip = clip;
        let outer_opacity = self.opacity;
        if let Some(opacity) = bound.and_then(|bound| bound.opacity) {
            self.opacity = opacity;
        }
        let content = [
            origin[0] + frame.content_at[0],
            origin[1] + frame.content_at[1],
        ];
        self.backgrounds(window, container, origin, frame, clip);
        let scroll = self.content_scroll(window, container);
        let offset = self.scroll_offset(window, &scroll);
        let clips = container
            .clipping
            .unwrap_or(!container.backgrounds.is_empty());
        let clip = if clips {
            intersect(
                clip,
                [
                    content[0],
                    content[1],
                    content[0] + frame.content_size[0],
                    content[1] + frame.content_size[1],
                ],
            )
        } else {
            clip
        };
        let clip = match own_clip {
            Some(own) => intersect(clip, own),
            None => clip,
        };
        let root = self.top.is_none();
        let (first_part, first_holder, first_region) =
            (self.parts.len(), self.holders.len(), self.regions.len());
        let moved = [
            content[0].wrapping_add(offset[0]),
            content[1].wrapping_add(offset[1]),
        ];
        for (index, child) in window.children.iter().enumerate() {
            if !self.shown(child) && !matches!(child.fields, Fields::Scrollbar(_)) {
                continue;
            }
            if root {
                self.top = Some(index);
            }
            if let Err(refusal) = self.child(child, moved, frame.content_size, clip) {
                self.refusals.push(refusal);
            }
        }
        if root {
            self.top = None;
        }
        self.opacity = outer_opacity;
        if self.bindings.regions {
            let at = moved;
            let outer = origin;
            for region in &mut self.regions[first_region..] {
                if let Some(hit) = &mut region.hit {
                    hit.inherit(container.scale, at);
                    hit.inherit(1.0, outer);
                }
            }
            for holder in &mut self.holders[first_holder..] {
                if let Some(parents) = &mut holder.hit_scales {
                    parents.insert(0, (container.scale, at));
                    parents.insert(0, (1.0, outer));
                }
            }
        }
        if container.scale != 1.0 {
            let k = container.scale;
            let at = moved.map(|value| value as f32);
            let scale =
                |point: [f32; 2]| [0, 1].map(|axis| at[axis] + k * (point[axis] - at[axis]));
            for part in &mut self.parts[first_part..] {
                super::native_shader::draw::transform(
                    part,
                    k,
                    [at[0] * (1.0 - k), at[1] * (1.0 - k)],
                );
                let geometry = match &mut part.kind {
                    PartKind::Sprite { geometry, .. }
                    | PartKind::Cell { geometry, .. }
                    | PartKind::Progress { geometry, .. }
                    | PartKind::TextSprite { geometry, .. } => geometry,
                    PartKind::Animated(animated) => &mut animated.geometry,
                    PartKind::Text {
                        glyphs,
                        scale: glyph_scale,
                        ..
                    } => {
                        for glyph in glyphs {
                            glyph.at = scale(glyph.at);
                        }
                        *glyph_scale *= k;
                        continue;
                    }
                    PartKind::Inline { rect, .. } | PartKind::MaskedFlag { rect, .. } => {
                        let [x, y] = scale([rect[0], rect[1]]);
                        *rect = [x, y, rect[2] * k, rect[3] * k];
                        continue;
                    }
                    PartKind::Fan { centre, radius, .. } => {
                        *centre = scale(*centre);
                        *radius = radius.map(|value| value * k);
                        continue;
                    }
                    PartKind::Tiles { quads, .. } => {
                        for (rect, _) in quads {
                            let [x, y] = scale([rect[0], rect[1]]);
                            *rect = [x, y, rect[2] * k, rect[3] * k];
                        }
                        continue;
                    }
                };
                let [x, y] = scale([geometry.rect[0], geometry.rect[1]]);
                geometry.rect = [x, y, geometry.rect[2] * k, geometry.rect[3] * k];
                geometry.pivot = scale(geometry.pivot);
            }
            for holder in &mut self.holders[first_holder..] {
                holder.origin = scale(holder.origin);
                holder.scale *= k;
            }
            for region in &mut self.regions[first_region..] {
                let [x, y] = scale([region.rect[0], region.rect[1]]);
                region.rect = [x, y, region.rect[2] * k, region.rect[3] * k];
            }
        }
        if let Some(opacity) = bound.and_then(|bound| bound.opacity) {
            self.opacity = opacity;
        }
        let drawn =
            self.container_scrollbars(window, container, &scroll, origin, frame, outer_clip);
        self.opacity = outer_opacity;
        drawn?;
        Ok(true)
    }

    fn backgrounds(
        &mut self,
        window: &Element,
        container: &native::Container,
        origin: [i32; 2],
        frame: Frame,
        clip: Option<Clip>,
    ) {
        let margin = container.background_margin;
        let [left, top, right, bottom] = margin_sides(&margin, frame.size);
        let moved = if [left, top, right, bottom].iter().any(|side| *side != 0) {
            [left, top]
        } else {
            [0, 0]
        };
        for (index, background) in container.backgrounds.iter().enumerate() {
            let Some(button) = background.button() else {
                continue;
            };
            let at = [
                origin[0] + button.position[0] + moved[0],
                origin[1] + button.position[1] + moved[1],
            ];
            self.sized = Some([
                frame.size[0] - (left + right) - (button.position[0] + moved[0]),
                frame.size[1] - (top + bottom) - (button.position[1] + moved[1]),
            ]);
            let previous_background = self.background_index.replace(index);
            let drawn = self.drag_button(background, button, button.frame, at, clip);
            self.background_index = previous_background;
            self.sized = None;
            if let Err(refusal) = drawn {
                self.refusals.push(Refusal {
                    detail: format!("background of {}: {}", window.path, refusal.detail),
                    ..refusal
                });
            }
        }
    }

    fn child(
        &mut self,
        child: &Element,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let bound = self.bound(child);
        let parent = bound
            .and_then(|bound| bound.layout_parent)
            .unwrap_or(parent);
        match &child.fields {
            Fields::Container(_) => {
                let frame = bound
                    .and_then(|bound| bound.window_frame)
                    .map_or_else(|| container_frame(child, Some(parent)), Ok)
                    .map_err(|why| Refusal::new(&child.path, RefusalKind::NoParent, why))?;
                let place = self
                    .bound(child)
                    .and_then(|bound| bound.position)
                    .unwrap_or(frame.at);
                let at = [origin[0] + place[0], origin[1] + place[1]];
                self.window(child, at, frame, clip)?;
            }
            Fields::Icon(icon) => self.icon(child, icon, origin, parent, clip)?,
            Fields::Button(button) if child.kind == Kind::Checkbox => {
                self.check_box(child, button, origin, parent, clip)?;
            }
            Fields::Button(button) => self.button(child, button, origin, parent, clip)?,
            Fields::TextBox(text_box) => self.text_box(child, text_box, origin, parent, clip)?,
            Fields::Grid(grid) => {
                if let Some(background) = &grid.background
                    && let Some(button) = background.button()
                {
                    let set = self.bound(child).and_then(|bound| bound.position);
                    let at = grid_origin(child, grid, parent, [0, 0], set)
                        .map_err(|why| Refusal::new(&child.path, RefusalKind::NoParent, why))?;
                    let (_, size, _) = grid_geometry(child, grid, parent)
                        .map_err(|why| Refusal::new(&child.path, RefusalKind::NoParent, why))?;
                    self.sized = Some(size);
                    let drawn = self.sprite_part(
                        background,
                        &button.sprite,
                        button.frame,
                        1.0,
                        0.0,
                        [
                            origin[0] + at[0] + button.position[0],
                            origin[1] + at[1] + button.position[1],
                        ],
                        Centre::None,
                        clip,
                    );
                    self.sized = None;
                    drawn?;
                }
                self.hold(child, origin, parent, clip, self.opacity, None)?;
            }
            Fields::List(_) => self.hold(child, origin, parent, clip, 1.0, None)?,
            Fields::PositionType(_) => self.hold(child, origin, parent, clip, self.opacity, None)?,
            Fields::ListBox(list) => self.list_box(child, list, origin, parent, clip)?,
            Fields::EditBox(edit) => self.edit_box(child, edit, origin, parent, clip)?,
            Fields::Scrollbar(bar) => self.scrollbar_child(child, bar, origin, clip)?,
            Fields::ExtendedScrollbar(bar) => {
                self.extended_child(child, bar, origin, parent, clip)?;
            }
            Fields::DropDown(drop) => self.drop_down(child, drop, origin, clip)?,
            Fields::Browser(_) if child.kind == Kind::CurveGraph => self.curve_graph(child)?,
            Fields::Browser(_) => self.browser(child)?,
            Fields::Window(_) | Fields::TextSprite(_) | Fields::Other => {
                return Err(Refusal::new(
                    &child.path,
                    RefusalKind::Reader,
                    format!("is a {}, which no container builds", child.kind.keyword()),
                ));
            }
        }
        Ok(())
    }

    fn hold(
        &mut self,
        element: &Element,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
        opacity: f32,
        placed: Option<[i32; 2]>,
    ) -> Result<(), Refusal> {
        let bound = self.bound(element);
        let set = bound.and_then(|bound| bound.position);
        let (at, size) = match &element.fields {
            Fields::Grid(grid) => {
                let (_, size, _) = grid_geometry(element, grid, parent)
                    .map_err(|why| Refusal::new(&element.path, RefusalKind::NoParent, why))?;
                let at = grid_origin(element, grid, parent, [0, 0], set)
                    .map_err(|why| Refusal::new(&element.path, RefusalKind::NoParent, why))?;
                (
                    at,
                    bound
                        .and_then(|bound| bound.grid)
                        .map_or(size, |bounds| bounds.extent),
                )
            }
            Fields::List(list) => (list_origin(list, parent, set), list.size),
            Fields::PositionType(place) => (set.unwrap_or(place.position), [0, 0]),
            Fields::ListBox(list) => (
                if let Some(place) = placed {
                    place
                } else {
                    widgets::list_box_place(
                        element,
                        list,
                        element.kind == Kind::SmoothListBox,
                        self.bindings.viewport,
                    )?
                },
                list.size,
            ),
            _ => {
                return Err(Refusal::new(
                    &element.path,
                    RefusalKind::Reader,
                    "is not a holder",
                ));
            }
        };
        if self.bindings.regions && element.kind != Kind::PositionType {
            self.regions.push(Region {
                path: element.path.clone(),
                owner: self.region_owner.clone(),
                background_index: None,
                scroll: None,
                slide: None,
                rect: [
                    (origin[0] + at[0]) as f32,
                    (origin[1] + at[1]) as f32,
                    size[0] as f32,
                    size[1] as f32,
                ],
                clip,
                hit: None,
                disabled: false,
            });
        }
        self.holders.push(Holder {
            path: element.path.clone(),
            child: self.top,
            content: parent,
            origin: origin.map(|value| value as f32),
            scale: 1.0,
            hit_scales: Some(Vec::new()),
            logical: origin,
            clip,
            opacity,
            before: self.parts.len(),
            position: self.bound(element).and_then(|bound| bound.position),
        });
        Ok(())
    }

    pub(super) fn info(&self, path: &str, name: &[u8]) -> Option<SpriteInfo> {
        self.info_ref(path, name).map(Cow::into_owned)
    }

    fn info_ref(&self, path: &str, name: &[u8]) -> Option<Cow<'a, SpriteInfo>> {
        looked_up_info(self.resources, &self.refused, path, name, false)
    }

    pub(super) fn resolved<'b>(&self, found: &'b Element) -> &'b Element {
        let mut refused = self.refused.borrow_mut();
        for refusal in reader_refusals(found) {
            if !refused.contains(&refusal) {
                refused.push(refusal);
            }
        }
        found
    }

    fn sprite(&mut self, path: &str, name: &[u8]) -> Option<(Vec<u8>, Cow<'a, SpriteInfo>)> {
        let found = drawn_sprite(name, |name| {
            looked_up_info(self.resources, &self.refused, path, name, false)
        });
        if found.is_none() {
            self.diagnostics.push(Diagnostic {
                path: path.to_owned(),
                kind: DiagnosticKind::MissingFallback,
                detail: format!(
                    "names {:?}, and the content defines neither it nor {}: no drawable is made",
                    lossy(name),
                    lossy(FALLBACK_SPRITE)
                ),
            });
        }
        found
    }

    fn frame_index(frame: i32, state: State, info: &SpriteInfo) -> i32 {
        match frame {
            0 => 0,
            frame if frame > 0 => frame - 1,
            _ => {
                let index = state.number() - 1;
                if index < info.frames.max(0) as u32 {
                    index as i32
                } else {
                    0
                }
            }
        }
    }

    fn drag_button(
        &mut self,
        element: &Element,
        button: &native::Button,
        frame: i32,
        at: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        let bound = self.bound(element);
        let pointer = self.bindings.pointer.as_ref();
        let state = if pointer.is_some_and(|pointer| pointer.pressed.as_deref() == Some(path)) {
            State::Down(self.pointer_since(bound, 2))
        } else if self.hovered.as_deref() == Some(path) {
            State::Over(self.pointer_since(bound, 3))
        } else {
            State::Up
        };
        if let State::Down(_) | State::Over(_) = state {
            self.pointed = Some(Pointed {
                path: path.clone(),
                down: matches!(state, State::Down(_)),
            });
        }
        if let Some((part, hit)) = self.state_part(
            element,
            &button.sprite,
            FrameSource::Own(frame),
            state,
            1.0,
            0.0,
            at,
            Centre::None,
            clip,
        )? && !button.always_transparent
        {
            self.hits.push(Hittable {
                path: path.clone(),
                part,
                hit,
                check_box: false,
                checked: false,
            });
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "the element, the sprite's name and frame, and its scale, rotation, placement and clip")]
    fn sprite_part(
        &mut self,
        element: &Element,
        name: &[u8],
        frame: i32,
        scale: f32,
        rotation: f32,
        at: [i32; 2],
        centre: Centre,
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        self.state_part(
            element,
            name,
            FrameSource::Own(frame),
            State::Up,
            scale,
            rotation,
            at,
            centre,
            clip,
        )
        .map(|_| ())
    }

    #[expect(clippy::too_many_arguments, reason = "the element, the sprite's name, frame and state, and its scale, rotation, placement and clip")]
    fn state_part(
        &mut self,
        element: &Element,
        name: &[u8],
        frame: FrameSource,
        state: State,
        scale: f32,
        rotation: f32,
        at: [i32; 2],
        centre: Centre,
        clip: Option<Clip>,
    ) -> Result<Option<(usize, Hit)>, Refusal> {
        let path = &element.path;
        let bound = self.bound(element);
        let name = made(bound, name, self.resources).map_or(name, |(sprite, _)| sprite);
        let texture = bound.and_then(|bound| bound.texture);
        let stretch = bound.and_then(|bound| bound.stretch);
        let bound_frame = bound.and_then(|bound| bound.frame);
        let bound_progress = bound.and_then(|bound| bound.progress);
        let bound_progress_float = bound.and_then(|bound| bound.progress_float);
        let bound_clock = bound.and_then(|bound| bound.clock);
        let bound_pulse = bound.and_then(|bound| bound.pulse);
        let bound_shield = bound.and_then(|bound| bound.shield);
        let tint = bound.and_then(|bound| bound.color);
        let Some((sprite, info)) = self.sprite(path, name) else {
            return Ok(None);
        };
        if matches!(info.kind, SpriteKind::Text { .. }) {
            self.text_sprite(
                element,
                &sprite,
                &info,
                None,
                -1,
                state,
                at,
                [0, 0],
                rotation,
                clip,
            )?;
            return Ok(None);
        }
        let effect = info
            .effect
            .clone()
            .map_err(|why| Refusal::new(path, RefusalKind::CustomEffect, why))?;
        let records = if info.animated && texture.is_none() {
            self.animation_records(element, &sprite, &info, state)?
        } else {
            Vec::new()
        };
        if texture.is_none()
            && let Some(why) = &info.missing
            && matches!(
                info.kind,
                SpriteKind::Plain | SpriteKind::FrameAnimated { .. } | SpriteKind::Tile { .. }
            )
        {
            self.diagnostics.push(Diagnostic {
                path: path.clone(),
                kind: DiagnosticKind::NoTexture,
                detail: format!(
                    "shows {}, which {why}: the sprite draw returns without drawing",
                    lossy(&sprite)
                ),
            });
            return Ok(None);
        }
        let info = match texture {
            Some(texture) => Cow::Owned(SpriteInfo {
                texture: texture.size,
                frames: 1,
                vertical: false,
                kind: SpriteKind::Plain,
                ..info.into_owned()
            }),
            None => info,
        };
        let extent = info.scaled_extent(scale);
        let offset = match centre {
            Centre::None => [0, 0],
            Centre::Position => [extent[0] / 2, extent[1] / 2],
            Centre::Unscaled => {
                let unscaled = info.scaled_extent(1.0);
                [unscaled[0] / 2, unscaled[1] / 2]
            }
            Centre::Anchor => match info.kind {
                SpriteKind::Progress { .. } => [0, 0],
                _ => info.anchor.map(|value| value as i32),
            },
        };
        let [x, y] = [(at[0] - offset[0]) as f32, (at[1] - offset[1]) as f32];
        let hit = matches!(
            &info.kind,
            SpriteKind::Plain | SpriteKind::FrameAnimated { .. }
        )
        .then(|| {
            self.bindings
                .viewport
                .zip(self.resources.physical_viewport())
        })
        .flatten()
        .map(|(viewport, physical)| {
            HitGeometry::sprite(
                [at[0].wrapping_sub(offset[0]), at[1].wrapping_sub(offset[1])],
                extent,
                [0, 1].map(|axis| {
                    (info.texture[axis] as f32 * scale) * (1.0 / info.divisors()[axis] as f32)
                }),
                info.hit,
                viewport,
                physical,
                clip,
            )
            .with_alpha(
                (texture.is_none() && info.hit == Hit::Alpha)
                    .then(|| self.resources.sprite_alpha(&sprite))
                    .flatten(),
                scale,
                {
                    let own = match frame {
                        FrameSource::Own(frame) | FrameSource::State(frame) => frame,
                    };
                    let selected = if matches!(info.kind, SpriteKind::FrameAnimated { .. }) {
                        bound_clock
                            .and_then(|(written, _)| written)
                            .unwrap_or(bound_frame.unwrap_or(own))
                    } else {
                        bound_frame.unwrap_or(own)
                    };
                    let index = Self::frame_index(selected, state, &info);
                    let extent = info.extent();
                    if info.vertical {
                        [0, index.wrapping_mul(extent[1])]
                    } else {
                        [index.wrapping_mul(extent[0]), 0]
                    }
                },
            )
        });
        let mut color = tint.unwrap_or([1.0; 4]);
        color[3] *= self.opacity;
        let frame_animated = matches!(info.kind, SpriteKind::FrameAnimated { .. });
        let (kind, mut shading) = match info.kind {
            SpriteKind::Plain => {
                let [fw, fh] = info.frame_size();
                let bottom = y + extent[1] as f32;
                let height = fh * scale * stretch.unwrap_or(1.0);
                let inset = (fh * scale - height) * 0.5;
                let geometry = Geometry {
                    rect: [x, bottom - inset - height, fw * scale, height],
                    rotation,
                    pivot: [x + fw * 0.5, bottom - fh * 0.5],
                    local: [0.0, inset, fw * scale, height],
                };
                let kind = match texture {
                    Some(texture) => PartKind::Cell {
                        texture: texture.texture,
                        masking: info.masked.then(|| sprite.clone()),
                        uv: texture.uv,
                        geometry,
                    },
                    None => {
                        let own = match frame {
                            FrameSource::Own(frame) => frame,
                            FrameSource::State(frame) => frame,
                        };
                        let index = Self::frame_index(bound_frame.unwrap_or(own), state, &info);
                        let ([du, dv], step) = info.frame_uv();
                        let offset = |index: i32| [index as f32 * step[0], index as f32 * step[1]];
                        let [ox, oy] = offset(index);
                        let [nx, ny] = offset(index.wrapping_add(1));
                        if records.is_empty() {
                            PartKind::Sprite {
                                sprite,
                                geometry,
                                uv: [0.0, 0.0, du, dv],
                                offsets: [ox, oy, nx, ny],
                                masked: info.masked,
                            }
                        } else {
                            PartKind::Animated(Box::new(Animated {
                                sprite,
                                geometry,
                                uv: [0.0, 0.0, du, dv],
                                offsets: [ox, oy, nx, ny],
                                effect: effect.file.clone(),
                                technique: state.technique(),
                                records: records,
                                masked: info.masked,
                            }))
                        }
                    }
                };
                (kind, shading(&effect, state.technique(), state.since()))
            }
            SpriteKind::Progress {
                size,
                steps,
                colors,
                horizontal,
                textured,
            } => {
                let own = match frame {
                    FrameSource::Own(frame) => frame,
                    FrameSource::State(frame) if frame > 0 => frame,
                    FrameSource::State(_) => state.number() as i32,
                };
                let value = match bound_progress_float {
                    Some(value) => value,
                    None => bound_progress.or(bound_frame).unwrap_or(own).min(steps),
                };
                let state_value = (value as f32 / steps as f32) * 1.0;
                let [w, h] = size.map(|value| value as f32);
                let uv = if horizontal {
                    [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]
                } else {
                    [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]
                };
                let times = |own: [f32; 4]| [0, 1, 2, 3].map(|index| own[index] * color[index]);
                let technique = if textured { "Texture" } else { "Color" };
                (
                    PartKind::Progress {
                        sprite,
                        geometry: Geometry {
                            rect: [x, y + 1.0, w, h],
                            rotation,
                            pivot: [x, y + h],
                            local: [0.0, -1.0, w, h],
                        },
                        uv,
                        state: state_value,
                        colors: [times(colors[0]), times(colors[1])],
                        textured,
                    },
                    shading(&effect, technique, 0.0),
                )
            }
            SpriteKind::CorneredTile {
                size,
                border,
                tiling_center,
                looping,
                rate,
            } => {
                let _ = rotation;
                let size = self.sized.unwrap_or(size);
                let own = match frame {
                    FrameSource::Own(frame) | FrameSource::State(frame) => frame,
                };
                let mut f = bound_frame.unwrap_or(own);
                if f != 0 && (f > info.frames || f < 1) {
                    f = 1;
                }
                if looping && rate.is_none() {
                    return Err(Refusal::new(
                        path,
                        RefusalKind::UndefinedRead,
                        format!(
                            "shows {}, a looping cornered tile with no animation rate, which \
                             its constructor leaves uninitialised and the pulse reads every \
                             frame: the heap block is not zeroed",
                            lossy(&sprite)
                        ),
                    ));
                }
                if let Some(why) = bound.and_then(|bound| bound.hang.as_ref()) {
                    return Err(Refusal::new(path, RefusalKind::Hang, why.clone()));
                }
                let alpha = if looping {
                    bound_pulse.ok_or_else(|| {
                        Refusal::new(
                            path,
                            RefusalKind::MissingInput,
                            format!(
                                "shows {}, a looping cornered tile whose alpha pulses on its \
                                 instance's clock: lay it out through its \
                                 lifecycle Instance",
                                lossy(&sprite)
                            ),
                        )
                    })?
                } else {
                    self.opacity
                };
                color[3] = alpha;
                let quads =
                    cornered_quads(size, border, info.texture, info.frames, f, tiling_center)
                        .into_iter()
                        .map(|([qx, qy, qw, qh], uv)| ([x + qx, y + qy, qw, qh], uv))
                        .collect();
                (
                    PartKind::Tiles {
                        sprite,
                        quads,
                        native_origin: [x, y],
                        cornered: true,
                    },
                    shading(&effect, state.technique(), state.since()),
                )
            }
            SpriteKind::Tile { size } => {
                let [w, h] = size;
                let bottom = y + info.texture[1] as f32;
                let nu = (w / info.texture[0]) as f32;
                let nv = (h / info.texture[1]) as f32;
                (
                    if records.is_empty() {
                        PartKind::Tiles {
                            sprite,
                            native_origin: [x, bottom],
                            cornered: false,
                            quads: vec![(
                                [x, bottom - h as f32, w as f32, h as f32],
                                [0.0, 0.0, nu, nv],
                            )],
                        }
                    } else {
                        PartKind::Animated(Box::new(Animated {
                            sprite,
                            geometry: Geometry {
                                rect: [x, bottom - h as f32, w as f32, h as f32],
                                local: [0., 0., w as f32, h as f32],
                                pivot: [x, bottom],
                                rotation: 0.,
                            },
                            uv: [0., 0., nu, nv],
                            offsets: [0.; 4],
                            effect: effect.file.clone(),
                            technique: state.technique(),
                            records: records,
                            masked: info.masked,
                        }))
                    },
                    shading(&effect, state.technique(), state.since()),
                )
            }
            SpriteKind::Resizeable => return Ok(None),
            SpriteKind::FrameAnimated { play_on_show, .. } => {
                if let Some(why) = bound.and_then(|bound| bound.hang.as_ref()) {
                    return Err(Refusal::new(path, RefusalKind::Hang, why.clone()));
                }
                let own = match frame {
                    FrameSource::Own(frame) | FrameSource::State(frame) => frame,
                };
                let (written, elapsed) = match bound_clock {
                    Some(clock) => clock,
                    None if play_on_show => {
                        return Err(Refusal::new(
                            path,
                            RefusalKind::MissingInput,
                            format!(
                                "shows {}, a frameAnimatedSpriteType that plays when shown \
                                 (play_on_show) on its instance's clock: lay it out through \
                                 its lifecycle Instance",
                                lossy(&sprite)
                            ),
                        ));
                    }
                    None => (None, 0.0),
                };
                let clock_frame = written.unwrap_or(bound_frame.unwrap_or(own));
                let animation = lifecycle::animation_time(clock_frame, elapsed, &info.kind);
                let [fw, fh] = info.frame_size();
                let bottom = y + extent[1] as f32;
                let geometry = Geometry {
                    rect: [x, bottom - fh * scale, fw * scale, fh * scale],
                    rotation,
                    pivot: [x + fw * 0.5, bottom - fh * 0.5],
                    local: [0.0, 0.0, fw * scale, fh * scale],
                };
                let index = Self::frame_index(clock_frame, state, &info);
                let ([du, dv], step) = info.frame_uv();
                let offset = |index: i32| [index as f32 * step[0], index as f32 * step[1]];
                let [ox, oy] = offset(index);
                let [nx, ny] = offset(index.wrapping_add(1));
                let mut shaded = shading(&effect, state.technique(), state.since());
                shaded.animation = animation;
                (
                    PartKind::Sprite {
                        sprite,
                        geometry,
                        uv: [0.0, 0.0, du, dv],
                        offsets: [ox, oy, nx, ny],
                        masked: info.masked,
                    },
                    shaded,
                )
            }
            SpriteKind::MaskedShield { mask, small } => {
                let [ow, oh] = info.texture.map(|value| value as f32);
                let [mw, mh] = mask.map(|value| value as f32);
                let hx = (ow - mw) * 0.5;
                let hy = (oh - mh) * 0.5;
                let bottom = y + oh;
                let index = bound_shield.unwrap_or(0);
                if let Some((flag, coords)) = self
                    .resources
                    .shield_flag(small, index)
                    .map_err(|why| Refusal::new(path, RefusalKind::MissingInput, why))?
                {
                    self.parts.push(Part {
                        path: path.clone(),
                        child: self.top,
                        background_index: self.background_index,
                        kind: PartKind::MaskedFlag {
                            sprite: sprite.clone(),
                            flag: flag.texture,
                            flag_coords: coords,
                            rect: [x + hx, bottom + 1.0 - (mh + hy), mw, mh],
                            native_origin: [x, bottom],
                        },
                        clip,
                        color,
                        shading: Self::kept(
                            shading(&effect, state.technique(), state.since()),
                            bound,
                            state,
                        ),
                        native: None,
                    });
                }
                let overlay = self
                    .resources
                    .effect(super::native_resources::SHIELD_OVERLAY_EFFECT)
                    .map_err(|why| Refusal::new(path, RefusalKind::CustomEffect, why))?;
                (
                    PartKind::Tiles {
                        sprite,
                        native_origin: [x, bottom],
                        cornered: false,
                        quads: vec![([x, bottom + 1.0 - oh, ow, oh], [0.0, 0.0, 1.0, 1.0])],
                    },
                    Shading {
                        clocked: false,
                        ..shading(&overlay, "Up", 0.0)
                    },
                )
            }
            SpriteKind::PieChart { size: None } => {
                return Err(Refusal::new(
                    path,
                    RefusalKind::UndefinedRead,
                    format!(
                        "shows {}, a PieChartType without size, which its constructor leaves \
                         uninitialised and the chart reads when sizing itself: the heap block \
                         is not zeroed",
                        lossy(&sprite)
                    ),
                ));
            }
            SpriteKind::PieChart { .. } | SpriteKind::LineChart { .. } => {
                if self.bindings.regions {
                    self.regions.push(Region {
                        path: path.clone(),
                        owner: self.region_owner.clone(),
                        background_index: self.background_index,
                        scroll: None,
                        slide: None,
                        rect: [x, y, extent[0] as f32, extent[1] as f32],
                        clip,
                        hit: None,
                        disabled: matches!(state, State::Disable),
                    });
                }
                return Ok(None);
            }
            SpriteKind::CircularProgress {
                size,
                rotation: turn,
                amount,
            } => {
                let raw = match bound_progress {
                    Some(percent) => [100i32.wrapping_sub(percent), percent],
                    None => {
                        let share = amount * 100.0;
                        [truncate_f32(100.0 - share), truncate_f32(share)]
                    }
                };
                let sum = raw[0].wrapping_add(raw[1]);
                let first = truncate_f32(raw[0] as f32 / sum as f32 * 100.0 + 0.5);
                let weights = [first, 100i32.wrapping_sub(first).max(0)];
                let [w, h] = [size, size].map(|value| value as f32 * scale);
                (
                    PartKind::Fan {
                        sprite,
                        centre: [x, y + size as f32],
                        radius: [w, h],
                        rotation: turn,
                        weights,
                    },
                    shading(&effect, "CircularProgressBar", 0.0),
                )
            }
            SpriteKind::Text { .. } => unreachable!("drawn above"),
        };
        if shading.compiled.is_some() && !frame_animated && shading.clocked {
            shading.animation = self.effect_clock(bound, state);
        }
        let shading = Self::kept(shading, bound, state);
        if let Some(rect) = part_rect(&kind)
            && self.bindings.regions
        {
            self.regions.push(Region {
                path: path.clone(),
                owner: self.region_owner.clone(),
                background_index: self.background_index,
                scroll: None,
                slide: None,
                rect,
                clip,
                hit,
                disabled: matches!(state, State::Disable),
            });
        }
        self.parts.push(Part {
            path: path.clone(),
            child: self.top,
            background_index: self.background_index,
            kind,
            clip,
            color,
            shading,
            native: None,
        });
        Ok(Some((self.parts.len() - 1, info.hit)))
    }

    fn kept(mut shading: Shading, bound: Option<&Bound>, state: State) -> Shading {
        if shading.fixture.is_none() && shading.clocked {
            shading.since = bound
                .and_then(|bound| bound.effect_since)
                .unwrap_or(state.since());
        }
        shading
    }

    fn icon(
        &mut self,
        element: &Element,
        icon: &native::Icon,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let anchor = control_anchor(icon.orientation, parent);
        let moved = self.offset(element);
        let at = [
            origin[0] + truncate_f32(icon.position[0]) + anchor[0] + moved[0],
            origin[1] + truncate_f32(icon.position[1]) + anchor[1] + moved[1],
        ];
        let centre = if icon.center_on_anchor {
            Centre::Anchor
        } else if icon.center_position {
            Centre::Position
        } else {
            Centre::None
        };
        let bound = self.bound(element);
        let (frame, rotation) = match made(bound, &icon.sprite, self.resources) {
            Some((_, SpriteSetter::Icon)) => (-1, 0.0),
            Some((_, SpriteSetter::ButtonStandard)) => (icon.frame.max(1), 0.0),
            _ => (icon.frame, icon.rotation),
        };
        let rotation = bound.and_then(|bound| bound.rotation).unwrap_or(rotation);
        let state = if bound.and_then(|bound| bound.enabled) == Some(false) {
            State::Disable
        } else {
            State::Up
        };
        self.state_part(
            element,
            &icon.sprite,
            FrameSource::Own(frame),
            state,
            bound.and_then(|bound| bound.scale).unwrap_or(icon.scale),
            rotation,
            at,
            centre,
            clip,
        )
        .map(|_| ())
    }

    fn button(
        &mut self,
        element: &Element,
        button: &native::Button,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        let bound = self.bound(element);
        let sprite = match made(bound, &button.sprite, self.resources) {
            Some((sprite, _)) => sprite,
            None => button.sprite.as_slice(),
        };
        let text = self.shown_text(element, &button.text);
        let has_font = !button.font.is_empty();
        if sprite.is_empty() && !has_font {
            return Err(Refusal::new(
                path,
                RefusalKind::UndefinedRead,
                "names no sprite and no font, so its constructor calls through its drawable \
                 before assigning it: the game reads an uninitialised pointer",
            ));
        }
        let anchor = control_anchor(button.orientation.unwrap_or_default(), parent);
        let moved = self.offset(element);
        let at = [
            origin[0] + button.position[0] + anchor[0] + moved[0],
            origin[1] + button.position[1] + anchor[1] + moved[1],
        ];
        let enabled = bound
            .and_then(|bound| bound.enabled)
            .unwrap_or(button.enabled);
        let pointer = self.bindings.pointer.as_ref();
        let state = if !enabled {
            State::Disable
        } else if pointer.is_some_and(|pointer| pointer.pressed.as_deref() == Some(path)) {
            State::Down(self.pointer_since(bound, 2))
        } else if self.hovered.as_deref() == Some(path) {
            State::Over(self.pointer_since(bound, 3))
        } else {
            State::Up
        };
        if let State::Down(_) | State::Over(_) = state {
            self.pointed = Some(Pointed {
                path: path.clone(),
                down: matches!(state, State::Down(_)),
            });
        }
        let text_only = text_only(button, sprite, text.as_deref());
        let technique = [sprite, FALLBACK_SPRITE]
            .into_iter()
            .find_map(|name| self.info_ref(path, name))
            .and_then(|info| {
                let effect = info.effect.as_ref().ok()?;
                let name = match state {
                    State::Up => "TextUp",
                    State::Over(_) => "TextOver",
                    State::Disable => "TextDisable",
                    State::Down(_) => "TextDown",
                };
                effect.has_technique(name).then(|| (effect.clone(), name))
            });
        let standard = matches!(
            made(bound, &button.sprite, self.resources),
            Some((_, SpriteSetter::ButtonStandard))
        );
        let (own_frame, turn, place) = if standard {
            let raw = [
                origin[0] + button.position[0] + moved[0],
                origin[1] + button.position[1] + moved[1],
            ];
            (button.frame.max(1), 0.0, raw)
        } else {
            let frame = if button.frame > 0 { button.frame } else { -1 };
            (frame, button.rotation, at)
        };
        let rotation = bound.and_then(|bound| bound.rotation).unwrap_or(turn);
        if has_font
            && let Some(info) = self
                .info_ref(path, sprite)
                .filter(|info| matches!(info.kind, SpriteKind::Text { .. }))
        {
            let Some(text) = text else {
                return Ok(());
            };
            let centre = if button.center_position {
                let [w, h] = self.text_measure(element, &button.font, &text, button.border_size)?;
                [w / 2, h / 2]
            } else {
                [0, 0]
            };
            let format = if bound.and_then(|bound| bound.text.as_ref()).is_some() {
                text_sprites::format_value(button.format)
            } else {
                2
            };
            let frame = bound.and_then(|bound| bound.frame).unwrap_or(own_frame);
            if let Some(part) = self.text_sprite(
                element,
                sprite,
                &info,
                Some(button_bake(button, &text, format)),
                frame,
                state,
                place,
                centre,
                rotation,
                clip,
            )? && !button.always_transparent
            {
                self.hits.push(Hittable {
                    path: path.clone(),
                    part,
                    hit: info.hit,
                    check_box: false,
                    checked: false,
                });
            }
            return Ok(());
        }
        let mut frame_size = None;
        if !text_only {
            let drawn = [sprite, FALLBACK_SPRITE]
                .into_iter()
                .find_map(|name| self.info_ref(path, name));
            if let Some(info) = &drawn
                && matches!(info.kind, SpriteKind::Text { .. })
            {
                if has_font {
                    let frame = if button.frame > 0 { button.frame } else { -1 };
                    self.text_sprite(
                        element,
                        FALLBACK_SPRITE,
                        info,
                        Some(text_sprites::TextSpriteDraw {
                            text: "",
                            font: &button.font,
                            texture: text_sprites::BakeTexture::Type,
                            size: [0, 0],
                            border: [0, 0],
                            format: 2,
                            fixed: true,
                            special: true,
                            multiline: true,
                        }),
                        frame,
                        state,
                        at,
                        [0, 0],
                        rotation,
                        clip,
                    )?;
                }
                frame_size = Some([0, 0]);
            } else {
                frame_size = drawn.map(|info| info.extent());
            }
            let (frame, centre) = match (standard, button.center_position) {
                (true, true) => (FrameSource::Own(own_frame), Centre::Unscaled),
                (true, false) => (FrameSource::Own(own_frame), Centre::None),
                (false, true) => (FrameSource::State(own_frame), Centre::Position),
                (false, false) => (FrameSource::State(own_frame), Centre::None),
            };
            if let Some((part, hit)) = self.state_part(
                element,
                sprite,
                frame,
                state,
                bound.and_then(|bound| bound.scale).unwrap_or(button.scale),
                rotation,
                place,
                centre,
                clip,
            )? && !button.always_transparent
            {
                self.hits.push(Hittable {
                    path: path.clone(),
                    part,
                    hit,
                    check_box: false,
                    checked: false,
                });
            }
        }
        let Some(text) = text.filter(|text| has_font && !text.is_empty()) else {
            return Ok(());
        };
        let font = &button.font;
        let common = |size: [i32; 2], border: [i32; 2], orientation: Orientation| TextSpec {
            font,
            text: &text,
            position: at,
            size: size.map(|value| if value == 0 { 200 } else { value }),
            border,
            format: button.format,
            vertical_alignment: button.vertical_alignment,
            orientation,
            extent: parent,
            fixed_size: false,
            truncate: false,
            multiline: button.multiline,
            special: true,
            color_code: 0,
            technique: technique.clone(),
        };
        let spec = if text_only {
            common(button.size, [0, 0], button.orientation.unwrap_or_default())
        } else {
            let frame = frame_size.unwrap_or([0, 0]);
            common(
                [
                    frame[0] - button.border_size[0],
                    frame[1] - button.border_size[1],
                ],
                button.border_size,
                Orientation::UpperLeft,
            )
        };
        self.text(element, spec, clip)
    }

    fn check_box(
        &mut self,
        element: &Element,
        button: &native::Button,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        if self.opacity < 1.0 {
            return Ok(());
        }
        let bound = self.bound(element);
        let sprite = made(bound, &button.sprite, self.resources)
            .map_or(button.sprite.as_slice(), |(sprite, _)| sprite)
            .to_vec();
        let checked = bound.and_then(|bound| bound.checked).unwrap_or(false);
        let hovered = self.boxes.iter().any(|hovered| hovered == path);
        let state = if checked && hovered {
            State::Down(self.pointer_since(bound, 2))
        } else if checked {
            State::Down(bound.and_then(|bound| bound.effect_since).ok_or_else(|| {
                Refusal::new(
                    path,
                    RefusalKind::MissingInput,
                    "is checked by its code, and its Down effect runs on its drawable's clock \
                     from the first draw after the check: lay it out through its lifecycle \
                     Instance",
                )
            })?)
        } else if hovered {
            State::Over(self.pointer_since(bound, 3))
        } else {
            State::Up
        };
        let anchor = control_anchor(button.orientation.unwrap_or_default(), parent);
        let at = [
            origin[0] + button.position[0] + anchor[0],
            origin[1] + button.position[1] + anchor[1],
        ];
        if !button.font.is_empty() {
            match self.info(path, &sprite) {
                None => return Ok(()),
                Some(info) if matches!(info.kind, SpriteKind::Text { .. }) => {
                    let Some(text) = self.shown_text(element, &button.text) else {
                        return Ok(());
                    };
                    if let Some(part) = self.text_sprite(
                        element,
                        &sprite,
                        &info,
                        Some(text_sprites::TextSpriteDraw {
                            text: &text,
                            font: &button.font,
                            texture: text_sprites::BakeTexture::Type,
                            size: [0, 0],
                            border: [0, 0],
                            format: 0,
                            fixed: true,
                            special: true,
                            multiline: true,
                        }),
                        -1,
                        state,
                        at,
                        [0, 0],
                        0.0,
                        clip,
                    )? {
                        self.hits.push(Hittable {
                            path: path.clone(),
                            part,
                            hit: info.hit,
                            check_box: true,
                            checked,
                        });
                    }
                    return Ok(());
                }
                Some(_) => {}
            }
        }
        if let Some((part, hit)) = self.state_part(
            element,
            &sprite,
            FrameSource::State(-1),
            state,
            1.0,
            0.0,
            at,
            Centre::None,
            clip,
        )? {
            self.hits.push(Hittable {
                path: path.clone(),
                part,
                hit,
                check_box: true,
                checked,
            });
        }
        Ok(())
    }

    fn text_box(
        &mut self,
        element: &Element,
        text_box: &native::TextBox,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let graphics_origin = origin;
        let offset = self.offset(element);
        let origin = [0, 1].map(|axis| origin[axis].wrapping_add(offset[axis]));
        let size = [text_box.max_width, text_box.max_height]
            .map(|value| if value == 0 { 200 } else { value as i32 });
        let anchor = control_anchor(text_box.orientation, parent);
        let shown = self.shown_text(element, &text_box.text);
        if self.bindings.regions {
            let hit = if text_box.always_transparent {
                None
            } else {
                let measured = self.text_size(
                    element,
                    &text_box.font,
                    shown.as_deref(),
                    TextObject::of(text_box),
                )?;
                self.bindings.viewport.map(|viewport| {
                    HitGeometry::text(
                        graphics_origin,
                        [0, 1].map(|axis| {
                            text_box.position[axis]
                                .wrapping_add(anchor[axis])
                                .wrapping_add(offset[axis])
                        }),
                        size,
                        measured[1],
                        viewport,
                        clip,
                    )
                })
            };
            self.regions.push(Region {
                path: element.path.clone(),
                owner: self.region_owner.clone(),
                background_index: None,
                scroll: None,
                slide: None,
                rect: [
                    origin[0]
                        .wrapping_add(text_box.position[0])
                        .wrapping_add(anchor[0]) as f32,
                    origin[1]
                        .wrapping_add(text_box.position[1])
                        .wrapping_add(anchor[1]) as f32,
                    size[0] as f32,
                    size[1] as f32,
                ],
                clip,
                hit,
                disabled: false,
            });
        }
        let mut scroll = 0.0f32;
        let mut text_clip = clip;
        let mut scrollbar = None;
        if let Some((plain, drawn, offset)) =
            self.text_scrollbar(element, text_box, self.bound(element))?
        {
            scroll = offset;
            if drawn {
                let anchor = control_anchor(text_box.orientation, parent);
                let [x, y] = [
                    origin[0] + text_box.position[0] + anchor[0],
                    origin[1] + text_box.position[1] + anchor[1],
                ];
                let [bx, by] = text_box.border_size;
                text_clip = Some([x + bx, y + by, x + size[0] - bx, y + size[1] - by]);
                scrollbar = Some(plain);
            }
        }
        if let Some(text) = &shown {
            self.text(
                element,
                TextSpec {
                    font: &text_box.font,
                    text,
                    position: [
                        origin[0] + text_box.position[0],
                        origin[1]
                            + if scrollbar.is_some() || scroll != 0.0 {
                                truncate_f32(text_box.position[1] as f32 + scroll)
                            } else {
                                text_box.position[1]
                            },
                    ],
                    size,
                    border: text_box.border_size,
                    format: text_box.format,
                    vertical_alignment: text_box.vertical_alignment,
                    orientation: text_box.orientation,
                    extent: parent,
                    fixed_size: text_box.fixed_size,
                    truncate: text_box.truncate,
                    multiline: text_box.multiline,
                    special: !text_box.ignore_special_chars,
                    color_code: text_box.text_color_code,
                    technique: None,
                },
                text_clip,
            )?;
        }
        if let Some(plain) = scrollbar {
            let node = plain.node(origin);
            let outer = self.opacity;
            self.opacity = 1.0;
            let drawn = self.plain_parts(&plain, element, node, clip, true);
            self.opacity = outer;
            drawn?;
        }
        Ok(())
    }

    fn text_layout(
        &self,
        element: &Element,
        font: &native_text::Font,
        sprite: &dyn Fn(&[u8]) -> Option<native_text::IconSprite>,
        chars: &[char],
        limits: Limits,
    ) -> Result<native_text::Laid, String> {
        if self
            .bound(element)
            .is_some_and(|bound| bound.resolved_flags)
        {
            native_text::lay_out_resolved_flags(font, sprite, chars, limits, &|text| {
                self.resources.resolved_text_flag_key(text)
            })
        } else {
            native_text::lay_out(font, sprite, chars, limits)
        }
    }

    fn prepared(
        &self,
        element: &Element,
        font: &native_text::Font,
        chars: &[char],
        object: TextObject,
    ) -> Result<Vec<char>, Refusal> {
        if !object.fixed_size && object.multiline {
            return Ok(chars.to_vec());
        }
        let [width, height] = object.size;
        let resources = self.resources;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        Ok(self
            .text_layout(
                element,
                font,
                &sprite,
                chars,
                Limits {
                    width,
                    height,
                    border: object.border,
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: object.truncate,
                    multiline: object.multiline,
                    special: object.special,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?
            .text)
    }

    pub(super) fn text_size(
        &self,
        element: &Element,
        font: &[u8],
        text: Option<&str>,
        object: TextObject,
    ) -> Result<[i32; 2], Refusal> {
        let font = text_font(self.resources, element, font)?;
        let [width, _] = object.size;
        let Some(text) = text else {
            return Ok([width, 0]);
        };
        let chars: Vec<char> = text.chars().collect();
        let prepared = self.prepared(element, font, &chars, object)?;
        let resources = self.resources;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        let [bx, by] = object.border;
        let laid = self
            .text_layout(
                element,
                font,
                &sprite,
                &prepared,
                Limits {
                    width,
                    height: i32::MAX,
                    border: object.border,
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: true,
                    special: true,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?;
        let widest = laid
            .widths
            .iter()
            .fold(0, |widest, width| widest.max(*width));
        let tall = laid
            .heights
            .iter()
            .fold(0i32, |sum, height| sum.wrapping_add(*height));
        let across = widest.wrapping_add(bx.wrapping_mul(2));
        Ok([
            if across < width { width } else { across },
            tall.wrapping_add(by.wrapping_mul(2)),
        ])
    }

    fn text_height(
        &self,
        element: &Element,
        font: &[u8],
        text: Option<&str>,
        object: TextObject,
    ) -> Result<i32, Refusal> {
        let font = text_font(self.resources, element, font)?;
        let Some(text) = text else {
            return Ok(0);
        };
        let chars: Vec<char> = text.chars().collect();
        let prepared = self.prepared(element, font, &chars, object)?;
        let resources = self.resources;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        let [bx, by] = object.border;
        let laid = self
            .text_layout(
                element,
                font,
                &sprite,
                &prepared,
                Limits {
                    width: object.size[0],
                    height: i32::MAX,
                    border: [bx.max(0), by.max(0)],
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: true,
                    special: true,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?;
        Ok(laid
            .heights
            .iter()
            .fold(0i32, |sum, height| sum.wrapping_add(*height)))
    }
    fn text(
        &mut self,
        element: &Element,
        spec: TextSpec,
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        let path = &element.path;
        if spec.text.is_empty() {
            return Ok(());
        }
        let resources = self.resources;
        let font = text_font(self.resources, element, spec.font)?;
        let facts = &font.facts;
        let effect = self
            .text_effect()
            .map_err(|why| Refusal::new(path, RefusalKind::CustomEffect, why))?;
        let text_shading = match &spec.technique {
            Some((effect, name)) => shading(effect, name, 0.0),
            None => shading(&effect, "Text", 0.0),
        };
        let global = resources.text_colors();
        let packed = facts.color;
        let mut own = [
            (packed >> 16 & 0xFF) as f32 / 255.0,
            (packed >> 8 & 0xFF) as f32 / 255.0,
            (packed & 0xFF) as f32 / 255.0,
            (packed >> 24) as f32 / 255.0,
        ];
        let colors = native_text::Colors {
            own,
            font: &facts.text_colors,
            global,
        };
        if spec.color_code != 0 {
            let code = char::from(spec.color_code);
            if code == '!' {
                own = [1.0; 4];
            } else if let Some(found) = colors.lookup(code) {
                own = found;
            }
        }
        if let Some(tint) = self.bound(element).and_then(|bound| bound.color) {
            own = tint;
        }
        let colors = native_text::Colors { own, ..colors };
        let icons_add_height = facts.icons_add_height;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        let refused = |why: String| Refusal::new(path, RefusalKind::DivideByZero, why);
        let chars: Vec<char> = spec.text.chars().collect();
        let [width, height] = spec.size;
        let [bx, by] = spec.border;
        let shown = self.prepared(
            element,
            font,
            &chars,
            TextObject {
                size: spec.size,
                border: spec.border,
                fixed_size: spec.fixed_size,
                multiline: spec.multiline,
                truncate: spec.truncate,
                special: spec.special,
            },
        )?;
        let laid = self
            .text_layout(
                element,
                font,
                &sprite,
                &shown,
                Limits {
                    width,
                    height: i32::MAX,
                    border: [bx.max(0), by.max(0)],
                    newlines: true,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: true,
                    special: spec.special,
                    icons_add_height,
                },
            )
            .map_err(refused)?;
        let [x, y] = spec.position;
        let [ew, eh] = spec.extent;
        let orientation = match spec.orientation {
            Orientation::UpperLeft | Orientation::CenterLeft | Orientation::CenterRight => 0,
            Orientation::UpperRight => 1,
            Orientation::LowerLeft => 2,
            Orientation::LowerRight => 3,
            Orientation::Center => 4,
            Orientation::CenterUp => 5,
            Orientation::CenterDown => 6,
        };
        let mut top = match orientation {
            2 | 3 | 6 => eh + y + by,
            4 => eh / 2 + y + by,
            _ => y + by,
        };
        let base = match orientation {
            1 | 3 => x + ew,
            4..=6 => ew / 2 + x + bx,
            _ => x,
        };
        let total: i32 = laid.heights.iter().sum();
        top += match spec.vertical_alignment {
            VerticalAlignment::Top => 0,
            VerticalAlignment::Center => ((height - by) - total) / 2,
            VerticalAlignment::Bottom => height - by - total,
        };
        let centred = orientation >= 4;
        let format = spec.format;
        let start = move |_: usize, line: i32| match format {
            TextFormat::Left if centred => base,
            TextFormat::Left => base + bx,
            TextFormat::Center if centred => base + width / 2 - line / 2,
            TextFormat::Center => base + bx + (width / 2 - line / 2),
            TextFormat::Right => base + width - line,
        };
        let pieces = native_text::draw(
            font,
            &sprite,
            &laid,
            spec.special,
            &Placement { top, start: &start },
            &colors,
        )
        .map_err(|why| {
            let kind = if why.contains("divides by") {
                RefusalKind::DivideByZero
            } else {
                RefusalKind::UndefinedRead
            };
            Refusal::new(path, kind, why)
        })?;
        let modulate = [1.0, 1.0, 1.0, self.opacity];
        let (projection, font_viewport) =
            text_projection(path, &facts.viewport, self.bindings.viewport)?;
        let text_part = |glyphs: Vec<DrawnGlyph>| Part {
            path: path.clone(),
            child: None,
            background_index: None,
            kind: PartKind::Text {
                font: font.facts.name.clone(),
                glyphs,
                scale: 1.0,
                modulate,
                border_color: font.facts.border_color,
                projection,
                font_viewport,
            },
            clip,
            color: [1.0; 4],
            shading: text_shading.clone(),
            native: None,
        };
        let mut glyphs = Vec::new();
        let mut flushed = false;
        for piece in pieces {
            let (source, rect, uv) = match piece {
                Piece::Glyph { ch, at, color } => {
                    let glyph = drawn_glyph(&mut self.diagnostics, path, font, ch, at, color);
                    glyphs.push(glyph);
                    if glyphs.len() == GLYPHS_PER_FLUSH {
                        flushed = true;
                        let part = Part {
                            child: self.top,
                            ..text_part(std::mem::take(&mut glyphs))
                        };
                        self.parts.push(part);
                    }
                    continue;
                }
                Piece::Undefined(name) => {
                    self.diagnostics.push(Diagnostic {
                        path: path.clone(),
                        kind: DiagnosticKind::UndefinedTextIcon,
                        detail: format!(
                            "shows the inline icon {name:?}, which the content does not define \
                             as GFX_ and its name nor bare"
                        ),
                    });
                    continue;
                }
                Piece::Icon {
                    sprite,
                    at,
                    size,
                    uv,
                } => (
                    InlineSource::Sprite(sprite),
                    [at[0] as f32, at[1] as f32, size[0] as f32, size[1] as f32],
                    uv,
                ),
                Piece::Flag { tag, at, resolved } => {
                    let cell = if resolved {
                        resources.resolved_text_flag(tag.as_bytes())
                    } else {
                        resources.text_flag(&tag)
                    };
                    let Some(cell) =
                        cell.map_err(|why| Refusal::new(path, RefusalKind::MissingInput, why))?
                    else {
                        if resolved {
                            self.diagnostics.push(Diagnostic {
                                path: path.clone(),
                                kind: DiagnosticKind::NullTextFlag,
                                detail: format!(
                                    "resolved inline flag {tag} has no valid resident atlas record"
                                ),
                            });
                        }
                        continue;
                    };
                    (
                        InlineSource::Texture(cell.texture),
                        [at[0] as f32, at[1] as f32, 10.0, 7.0],
                        cell.uv,
                    )
                }
            };
            self.parts.push(Part {
                path: path.clone(),
                child: self.top,
                background_index: None,
                kind: PartKind::Inline {
                    source,
                    rect,
                    uv,
                    modulate,
                    projection,
                    font_viewport,
                },
                clip,
                color: [1.0; 4],
                shading: text_shading.clone(),
                native: None,
            });
        }
        if !glyphs.is_empty() || !flushed {
            let part = Part {
                child: self.top,
                ..text_part(glyphs)
            };
            self.parts.push(part);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refreshed {
    Unchanged,
    Above { flagged: bool },
    AtMost,
}

impl<'a> Walk<'a> {
    pub(super) fn text_bar(
        &self,
        element: &Element,
        text_box: &native::TextBox,
    ) -> Result<Option<(widgets::Plain<'a>, bool, f32)>, Refusal> {
        let Some(mut plain) = self.named_plain(element, &text_box.scrollbar_type)? else {
            return Ok(None);
        };
        let size = [text_box.max_width, text_box.max_height]
            .map(|value| if value == 0 { 200 } else { value as i32 });
        plain.resize(-1, size[1]);
        let mut flag = true;
        let authored = localised(self.resources, &text_box.text);
        self.text_refresh(
            element,
            text_box,
            &mut plain,
            &mut flag,
            authored.as_deref(),
        )?;
        plain.move_to([
            text_box.position[0].wrapping_add(size[0]),
            text_box.position[1],
        ]);
        plain.set_value(0);
        let high = plain.range.max as f32 / 100_000.0;
        let scroll = -plain.value.max(0.0).min(high);
        Ok(Some((plain, flag, scroll)))
    }

    pub(super) fn text_refresh(
        &self,
        element: &Element,
        text_box: &native::TextBox,
        plain: &mut widgets::Plain,
        flag: &mut bool,
        text: Option<&str>,
    ) -> Result<Refreshed, Refusal> {
        let height = [text_box.max_width, text_box.max_height]
            .map(|value| if value == 0 { 200 } else { value as i32 })[1];
        let lines = self.text_height(element, &text_box.font, text, TextObject::of(text_box))?;
        let mut range = i64::from(lines.wrapping_sub(height)) * 100_000;
        if range < 100_000 {
            range = 100_000;
        }
        if range == widgets::fixed(plain.range.max as f32 / 100_000.0) {
            return Ok(Refreshed::Unchanged);
        }
        let step = widgets::fixed(plain.range.step as f32 / 100_000.0);
        plain.set_range(widgets::Range::new(0, range, step));
        if 1.0 < range as f32 / 100_000.0 {
            let flagged = *flag;
            *flag = false;
            plain.place_slider();
            Ok(Refreshed::Above { flagged })
        } else {
            *flag = true;
            Ok(Refreshed::AtMost)
        }
    }

    pub(super) fn text_scrollbar(
        &self,
        element: &Element,
        text_box: &native::TextBox,
        bound: Option<&Bound>,
    ) -> Result<Option<(widgets::Plain<'a>, bool, f32)>, Refusal> {
        let Some((mut plain, mut flag, scroll)) = self.text_bar(element, text_box)? else {
            return Ok(None);
        };
        match bound.and_then(|bound| bound.text_bar) {
            Some((state, kept)) => {
                plain.restore(state);
                flag = kept;
            }
            None => {
                if let Some(text) = bound.and_then(|bound| bound.text.as_deref()) {
                    self.text_refresh(element, text_box, &mut plain, &mut flag, Some(text))?;
                }
            }
        }
        let shown = !flag && plain.range.max >= 100_000;
        let scroll = bound.and_then(|bound| bound.text_scroll).unwrap_or(scroll);
        Ok(Some((plain, shown, scroll)))
    }
}

fn text_projection(
    path: &str,
    written: &[Option<i32>; 2],
    viewport: Option<[i32; 2]>,
) -> Result<([f32; 2], Option<[i32; 2]>), Refusal> {
    if written.iter().all(Option::is_none) {
        return Ok(([1.0; 2], None));
    }
    let Some(viewport) = viewport else {
        return Err(Refusal::new(
            path,
            RefusalKind::MissingInput,
            "draws with a font whose viewport a component wrote, which projects against the GUI \
             viewport that the caller did not give"
                .to_owned(),
        ));
    };
    let mut projection = [1.0; 2];
    for axis in 0..2 {
        if let Some(value) = written[axis] {
            projection[axis] = viewport[axis] as f32 / value as f32;
        }
    }
    let font = [0, 1].map(|axis| written[axis].unwrap_or(viewport[axis]));
    Ok((projection, Some(font)))
}

fn drawn_glyph(
    diagnostics: &mut Vec<Diagnostic>,
    path: &str,
    font: &native_text::Font,
    ch: char,
    at: [i32; 2],
    color: [f32; 4],
) -> DrawnGlyph {
    let quad = font.quad(ch).expect("a drawn glyph is the font's own");
    if let Some(page) = &quad.null_page {
        diagnostics.push(Diagnostic {
            path: path.to_owned(),
            kind: DiagnosticKind::NullFontPage,
            detail: format!(
                "draws {ch:?} of font {} from page {:?}, whose texture does not load: the page \
                 keeps a 0 by 0 extent and the draw binds the null texture",
                font.name(),
                lossy(page)
            ),
        });
    }
    DrawnGlyph {
        ch,
        at: at.map(|value| value as f32),
        color,
        texture: quad.page.texture,
        size: quad.size,
        uv: quad.uv,
    }
}

pub const GLYPHS_PER_FLUSH: usize = 0xC00 / 6;

fn button_bake<'t>(
    button: &'t native::Button,
    text: &'t str,
    format: i32,
) -> text_sprites::TextSpriteDraw<'t> {
    text_sprites::TextSpriteDraw {
        text,
        font: &button.font,
        texture: text_sprites::BakeTexture::Type,
        size: [0, 0],
        border: button.border_size,
        format,
        fixed: true,
        special: true,
        multiline: button.multiline,
    }
}

pub fn drawable_extent(
    element: &Element,
    resources: &dyn Resources,
    bindings: &Bindings,
) -> Result<Option<[i32; 2]>, Refusals> {
    let walk = Walk::new(resources, bindings);
    let extent = walk.drawable_extent(walk.resolved(element));
    let mut refusals = walk.refused.take();
    match extent {
        Ok(extent) if refusals.is_empty() => Ok(extent),
        Ok(_) => Err(refusals),
        Err(refusal) => {
            refusals.push(refusal);
            Err(refusals)
        }
    }
}

impl Walk<'_> {
    pub(super) fn drawable_extent(&self, element: &Element) -> Result<Option<[i32; 2]>, Refusal> {
        let path = &element.path;
        let bound = self.bound(element);
        let (authored, scale, button) = match &element.fields {
            Fields::Icon(icon) => (&icon.sprite, icon.scale, None),
            Fields::Button(button) if element.kind == Kind::Checkbox => {
                (&button.sprite, 1.0, Some(button))
            }
            Fields::Button(button) => (&button.sprite, button.scale, Some(button)),
            _ => return Ok(None),
        };
        let name =
            made(bound, authored, self.resources).map_or(authored.as_slice(), |(sprite, _)| sprite);
        let font = button.filter(|button| !button.font.is_empty());
        let text = || match bound.and_then(|bound| bound.text.clone()) {
            Some(text) => Some(text),
            None => localised(self.resources, &button.expect("a button").text),
        };
        if let Some(button) = button
            && element.kind != Kind::Checkbox
        {
            if name.is_empty() && font.is_none() {
                return Err(Refusal::new(
                    path,
                    RefusalKind::UndefinedRead,
                    "names no sprite and no font, so its constructor calls through its drawable \
                     before assigning it: the game reads an uninitialised pointer",
                ));
            }
            let shown = if font.is_some() && name.is_empty() {
                text()
            } else {
                None
            };
            if font.is_some() && name.is_empty() && shown.as_deref() != Some("") {
                let [w, h] = button
                    .size
                    .map(|value| if value == 0 { 200 } else { value });
                return self
                    .text_size(
                        element,
                        &button.font,
                        shown.as_deref(),
                        TextObject {
                            size: [w, h],
                            border: [0, 0],
                            fixed_size: false,
                            multiline: button.multiline,
                            truncate: false,
                            special: true,
                        },
                    )
                    .map(Some);
            }
        }
        if let Some(button) = font
            && let Some(info) = self
                .info(path, name)
                .filter(|info| matches!(info.kind, SpriteKind::Text { .. }))
        {
            return self
                .bake_extent(element, button, name, &info, text())
                .map(Some);
        }
        if let Some(button) = font
            && element.kind == Kind::Checkbox
            && self.info(path, name).is_none()
        {
            return Err(Refusal::new(
                path,
                RefusalKind::MissingInput,
                format!(
                    "is a check box with font {:?} whose sprite {:?} the registry lacks: \
                     constructing it registers a new text sprite type whose size query reads a \
                     target this module does not establish",
                    lossy(&button.font),
                    lossy(name)
                ),
            ));
        }
        let Some(info) = [name, FALLBACK_SPRITE]
            .into_iter()
            .find_map(|name| self.info(path, name))
        else {
            return Ok(None);
        };
        if font.is_some() && matches!(info.kind, SpriteKind::Text { .. }) {
            return Ok(Some([0, 0]));
        }
        let info = match bound.and_then(|bound| bound.texture) {
            Some(texture) => SpriteInfo {
                texture: texture.size,
                frames: 1,
                vertical: false,
                kind: SpriteKind::Plain,
                ..info
            },
            None => info,
        };
        Ok(Some(info.scaled_extent(
            bound.and_then(|bound| bound.scale).unwrap_or(scale),
        )))
    }

    fn bake_extent(
        &self,
        element: &Element,
        button: &native::Button,
        name: &[u8],
        info: &SpriteInfo,
        text: Option<String>,
    ) -> Result<[i32; 2], Refusal> {
        let path = &element.path;
        let unknown = || {
            Refusal::new(
                path,
                RefusalKind::MissingInput,
                format!(
                    "shows text sprite {} with nothing baked, whose size query reads a target \
                     size this module does not establish",
                    lossy(name)
                ),
            )
        };
        let text = text.ok_or_else(unknown)?;
        let mut scratch = Walk::new(self.resources, self.bindings);
        let bound_text = self
            .bound(element)
            .and_then(|bound| bound.text.as_ref())
            .is_some();
        let draw = if element.kind == Kind::Checkbox {
            text_sprites::TextSpriteDraw {
                text: &text,
                font: &button.font,
                texture: text_sprites::BakeTexture::Type,
                size: [0, 0],
                border: [0, 0],
                format: 0,
                fixed: true,
                special: true,
                multiline: true,
            }
        } else {
            let format = if bound_text {
                text_sprites::format_value(button.format)
            } else {
                2
            };
            button_bake(button, &text, format)
        };
        let part = scratch.text_sprite(
            element,
            name,
            info,
            Some(draw),
            -1,
            State::Up,
            [0, 0],
            [0, 0],
            0.0,
            None,
        );
        self.refused.borrow_mut().extend(scratch.refused.take());
        let part = part?.ok_or_else(unknown)?;
        let PartKind::TextSprite { bake, .. } = &scratch.parts[part].kind else {
            unreachable!("a text sprite's part");
        };
        let frames = info.frames as u32;
        Ok([(bake.size[0] as u32 / frames) as i32, bake.size[1]])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameSource {
    Own(i32),
    State(i32),
}

fn text_font<'r>(
    resources: &'r dyn Resources,
    element: &Element,
    name: &[u8],
) -> Result<&'r native_text::Font, Refusal> {
    let font = resources.font(name).ok_or_else(|| {
        Refusal::new(
            &element.path,
            RefusalKind::NullRead,
            format!(
                "names font {:?}, and the content registers no bitmapfont: the font lookup \
                 returns null, which the text object constructor calls through",
                lossy(name)
            ),
        )
    })?;
    if let Some((kind, _)) = font.refusals.first() {
        return Err(Refusal::new(
            &element.path,
            *kind,
            format!(
                "draws in font {}, which is not taken as the game takes it: {}",
                font.name(),
                font.refusals
                    .iter()
                    .map(|(_, why)| why.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }
    Ok(font)
}

fn noted<'a>(
    refused: &RefCell<Refusals>,
    path: &str,
    name: &[u8],
    info: Option<Cow<'a, SpriteInfo>>,
) -> Option<Cow<'a, SpriteInfo>> {
    if let Some(info) = &info {
        let mut refused = refused.borrow_mut();
        for (kind, why) in &info.refusals {
            let refusal = Refusal::new(
                path,
                *kind,
                format!(
                    "shows {}, which is not taken as the game takes it: {why}",
                    lossy(name)
                ),
            );
            if !refused.contains(&refusal) {
                refused.push(refusal);
            }
        }
    }
    info
}

fn icon_sprite(
    resources: &dyn Resources,
    refused: &RefCell<Refusals>,
    path: &str,
    name: &[u8],
) -> Option<native_text::IconSprite> {
    looked_up(resources, refused, path, name, false).map(|info| {
        let frames = match info.kind {
            SpriteKind::Progress { .. } | SpriteKind::CorneredTile { .. } => 1,
            _ => info.frames,
        };
        (info.texture, frames)
    })
}

#[derive(Clone, Copy, Debug)]
struct TextObject {
    size: [i32; 2],
    border: [i32; 2],
    fixed_size: bool,
    multiline: bool,
    truncate: bool,
    special: bool,
}

impl TextObject {
    fn of(text_box: &native::TextBox) -> Self {
        Self {
            size: [text_box.max_width, text_box.max_height]
                .map(|value| if value == 0 { 200 } else { value as i32 }),
            border: text_box.border_size,
            fixed_size: text_box.fixed_size,
            multiline: text_box.multiline,
            truncate: text_box.truncate,
            special: !text_box.ignore_special_chars,
        }
    }
}

struct TextSpec<'a> {
    font: &'a [u8],
    text: &'a str,
    position: [i32; 2],
    size: [i32; 2],
    border: [i32; 2],
    format: TextFormat,
    vertical_alignment: VerticalAlignment,
    orientation: Orientation,
    extent: [i32; 2],
    fixed_size: bool,
    truncate: bool,
    multiline: bool,
    special: bool,
    color_code: u8,
    technique: Option<(Effect, &'static str)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assets {
    pub sprites: BTreeSet<Vec<u8>>,
    pub fonts: BTreeSet<Vec<u8>>,
    pub textures: BTreeSet<Vec<u8>>,
}

impl Assets {
    pub fn from_localisation(localisation: &Localisation) -> Self {
        let mut assets = Self::default();
        for (key, _) in localisation.iter() {
            assets.add_localised(localisation, key.as_bytes());
        }
        assets
    }

    pub fn add_text(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        self.add_chars(&chars);
    }

    pub fn add_localised(&mut self, localisation: &Localisation, key: &[u8]) {
        if let Some(chars) = shown_chars(localisation, key) {
            self.add_chars(&chars);
        }
    }

    fn add_chars(&mut self, chars: &[char]) {
        let mut index = 0;
        while index < chars.len() {
            match native_text::special(&chars[index..]) {
                Some((length, native_text::Special::Icon(name))) => {
                    let (names, _) = content_reader_localisation::localisation::icon_lookup(&name);
                    self.sprites.extend(names);
                    index += length;
                }
                Some((length, _)) => index += length,
                None => index += 1,
            }
        }
    }

    pub fn extend(&mut self, other: Assets) {
        self.sprites.extend(other.sprites);
        self.fonts.extend(other.fonts);
        self.textures.extend(other.textures);
    }
}

pub fn assets_in<'a>(
    element: &'a Element,
    gui_type: &dyn Fn(&[u8]) -> Option<&'a Element>,
    localisation: &Localisation,
) -> Assets {
    let mut assets = Assets::default();
    assets.sprites.insert(FALLBACK_SPRITE.to_vec());
    let mut seen = Vec::new();
    collect_assets(element, gui_type, localisation, &mut assets, &mut seen);
    assets
}

fn collect_assets<'a>(
    element: &'a Element,
    gui_type: &dyn Fn(&[u8]) -> Option<&'a Element>,
    localisation: &Localisation,
    assets: &mut Assets,
    seen: &mut Vec<*const Element>,
) {
    if seen.contains(&std::ptr::from_ref::<Element>(element)) {
        return;
    }
    seen.push(element);
    let mut sprite = |name: &[u8]| {
        if !name.is_empty() {
            assets.sprites.insert(name.to_vec());
        }
    };
    let mut named: Vec<&[u8]> = Vec::new();
    let mut found: Vec<&'a Element> = Vec::new();
    match &element.fields {
        Fields::Container(container) => {
            named.extend([
                container.vertical_scrollbar.as_slice(),
                container.horizontal_scrollbar.as_slice(),
            ]);
        }
        Fields::Icon(icon) => sprite(&icon.sprite),
        Fields::Button(button) => {
            sprite(&button.sprite);
            if !button.font.is_empty() {
                assets.fonts.insert(button.font.clone());
                assets.add_localised(localisation, &button.text);
            }
        }
        Fields::TextBox(text_box) => {
            assets.fonts.insert(text_box.font.clone());
            assets.add_localised(localisation, &text_box.text);
            named.push(&text_box.scrollbar_type);
        }
        Fields::ListBox(list) => {
            sprite(&list.background);
            named.push(&list.scrollbar_type);
        }
        Fields::EditBox(edit) => {
            assets.fonts.insert(edit.font.clone());
            assets.add_localised(localisation, &edit.text);
            assets.sprites.insert(EDIT_CARET.to_vec());
            if edit.flags() & 0x2 == 0 {
                assets.sprites.insert(Vec::new());
                if !edit.texture.is_empty() {
                    assets.textures.insert(edit.texture.clone());
                }
            }
        }
        Fields::Scrollbar(bar) => {
            for (name, _, _) in widgets::scrollbar_part_names(bar) {
                if let Ok(part) = widgets::scrollbar_part(element, bar, name, gui_type) {
                    found.push(part);
                }
            }
        }
        Fields::Browser(browser) => sprite(&browser.sprite),
        Fields::Window(_) => {
            if let Ok(Some(background)) = fixed::background(element, gui_type) {
                found.push(background);
            }
        }
        Fields::TextSprite(text) => {
            assets.sprites.insert(element.name_bytes().to_vec());
            assets.fonts.insert(text.font.clone());
            assets.add_localised(localisation, &text.text);
            if !text.texture.is_empty() {
                assets.textures.insert(text.texture.clone());
            }
        }
        _ => {}
    }
    for name in named {
        if !name.is_empty()
            && let Some(template) = gui_type(name)
        {
            found.push(template);
        }
    }
    for element in found.into_iter().chain(element.nested()) {
        collect_assets(element, gui_type, localisation, assets, seen);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ListEntry<'a> {
    Container(&'a Element),
    Fixed(&'a Element),
}

pub fn list_entry<'a>(
    path: &str,
    name: &[u8],
    gui_type: &dyn Fn(&[u8]) -> Option<&'a Element>,
) -> Result<ListEntry<'a>, Refusal> {
    let Some(template) = gui_type(name) else {
        return Err(Refusal::new(
            path,
            RefusalKind::MissingInput,
            format!(
                "makes a list box item of {:?}, which no template defines: it uses the GUI's \
                 own object, which no template describes",
                lossy(name)
            ),
        ));
    };
    match template.kind {
        Kind::Container => Ok(ListEntry::Container(template)),
        Kind::Window => Ok(ListEntry::Fixed(template)),
        _ => Err(Refusal::new(
            path,
            RefusalKind::TypeConfusion,
            format!(
                "makes a list box item of {}, a {}, which is passed to the fixed-window builder \
                 with no kind check: it reads window-template fields beyond the {}'s own \
                 layout",
                template.path,
                template.kind.keyword(),
                template.kind.keyword()
            ),
        )),
    }
}

#[derive(Clone, Copy)]
enum Centre {
    None,
    Position,
    Unscaled,
    Anchor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridLayout {
    pub origin: [i32; 2],
    pub relaid: [i32; 2],
    pub items: Vec<[i32; 2]>,
    pub extent: [i32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridBounds {
    adjust: [i32; 2],
    extent: [i32; 2],
}

#[cfg(any(test, feature = "test-support"))]
pub fn item_offset(window: &Element, viewport: [i32; 2]) -> Result<[i32; 2], String> {
    Ok(container_frame(window, Some(viewport))?.at)
}

fn next_slot(grid: &native::Grid, last: Option<[i32; 2]>) -> Option<[i32; 2]> {
    let [horizontal, vertical] = grid.max_slots;
    let Some([a, b]) = last else {
        return Some([0, 0]);
    };
    if grid.add_horizontal {
        let mut next = [a.wrapping_add(1), b];
        if horizontal > 0 && next[0] >= horizontal {
            next = [0, b.wrapping_add(1)];
        }
        (!(vertical > 0 && next[1] >= vertical)).then_some(next)
    } else {
        let mut next = [a, b.wrapping_add(1)];
        if vertical > 0 && next[1] >= vertical {
            next = [a.wrapping_add(1), 0];
        }
        (!(horizontal > 0 && next[0] >= horizontal)).then_some(next)
    }
}

fn slot_place(grid: &native::Grid, extent: [i32; 2], cell: [i32; 2], slot: [i32; 2]) -> [i32; 2] {
    let [gw, gh] = extent;
    let [cw, ch] = cell;
    let [col, row] = slot;
    match GridFormat::from_value(grid.format) {
        GridFormat::Up => [(gw - cw) / 2 + cw * col, ch * row],
        GridFormat::Down => [(gw - cw) / 2 + cw * col, gh - ch - ch * row],
        GridFormat::Left => [cw * row, (gh - ch) / 2 + ch * col],
        GridFormat::Right => [gw - cw - cw * row, (gh - ch) / 2 + ch * col],
        GridFormat::UpperLeft => [cw * col, ch * row],
        GridFormat::LowerLeft => [cw * col, gh - ch - ch * row],
        GridFormat::UpperRight => [gw - cw - cw * col, ch * row],
    }
}

type GridGeometry = ([i32; 2], [i32; 2], [i32; 2]);

pub fn grid_geometry(
    element: &Element,
    grid: &native::Grid,
    parent: [i32; 2],
) -> Result<GridGeometry, String> {
    let path = &element.path;
    let position = evaluate_position(
        &grid.position,
        Some(parent),
        format_args!("{path}.position"),
    )?;
    let size = evaluate_size(
        &grid.size,
        Some(parent),
        position,
        format_args!("{path}.size"),
    )?;
    let own = evaluate_position(&grid.position, Some(size), format_args!("{path}.position"))?;
    let cell = evaluate_size(
        &grid.slot_size,
        Some(size),
        own,
        format_args!("{path}.slotsize"),
    )?;
    Ok((position, size, cell))
}

pub fn grid_slots(
    element: &Element,
    parent: [i32; 2],
    count: usize,
    set: Option<[i32; 2]>,
) -> Result<Vec<[i32; 2]>, String> {
    let grid = element
        .grid()
        .ok_or_else(|| format!("{} is not a gridBoxType", element.path))?;
    let at = grid_origin(element, grid, parent, [0, 0], set)?;
    let (_, size, cell) = grid_geometry(element, grid, parent)?;
    let mut places = Vec::with_capacity(count);
    let mut last = None;
    for _ in 0..count {
        let Some(slot) = next_slot(grid, last) else {
            break;
        };
        last = Some(slot);
        let [x, y] = slot_place(grid, size, cell, slot);
        places.push([at[0] + x, at[1] + y]);
    }
    Ok(places)
}

pub fn finalize_grid(
    element: &Element,
    parent: [i32; 2],
    viewport: [i32; 2],
    items: &[&Element],
    set: Option<[i32; 2]>,
) -> Result<GridLayout, Refusal> {
    let path = &element.path;
    let grid = element
        .grid()
        .ok_or_else(|| Refusal::new(path, RefusalKind::Reader, "is not a gridBoxType"))?;
    let mut last = None;
    let positions = items.iter().map_while(|item| {
        let slot = next_slot(grid, last)?;
        last = Some(slot);
        Some((*item, slot))
    });
    finalize_grid_positions(element, parent, viewport, positions, set)
}

pub fn finalize_grid_positions<'a>(
    element: &Element,
    parent: [i32; 2],
    viewport: [i32; 2],
    items: impl IntoIterator<Item = (&'a Element, [i32; 2])>,
    set: Option<[i32; 2]>,
) -> Result<GridLayout, Refusal> {
    let path = &element.path;
    let grid = element
        .grid()
        .ok_or_else(|| Refusal::new(path, RefusalKind::Reader, "is not a gridBoxType"))?;
    let no_parent = |why: String| Refusal::new(path, RefusalKind::NoParent, why);
    let (_, size, cell) = grid_geometry(element, grid, parent).map_err(no_parent)?;
    let mut placed = Vec::new();
    for (item, slot) in items {
        let place = slot_place(grid, size, cell, slot);
        let frame = container_frame(item, Some(viewport))
            .map_err(|why| Refusal::new(&item.path, RefusalKind::NoParent, why))?;
        let offset = frame.at;
        let at = [
            place[0].wrapping_add(offset[0]),
            place[1].wrapping_add(offset[1]),
        ];
        placed.push((at, frame.size));
    }
    finalize_placed(element, grid, parent, size, &placed, set)
}

pub fn finalize_grid_resolved(
    element: &Element,
    parent: [i32; 2],
    items: &[([i32; 2], [i32; 2])],
    set: Option<[i32; 2]>,
) -> Result<GridLayout, Refusal> {
    let grid = element
        .grid()
        .ok_or_else(|| Refusal::new(&element.path, RefusalKind::Reader, "is not a gridBoxType"))?;
    let (_, size, _) = grid_geometry(element, grid, parent)
        .map_err(|why| Refusal::new(&element.path, RefusalKind::NoParent, why))?;
    finalize_placed(element, grid, parent, size, items, set)
}

fn finalize_placed(
    element: &Element,
    grid: &native::Grid,
    parent: [i32; 2],
    size: [i32; 2],
    items: &[([i32; 2], [i32; 2])],
    set: Option<[i32; 2]>,
) -> Result<GridLayout, Refusal> {
    let [mut min_x, mut min_y, mut max_x, mut max_y] = [0i32; 4];
    for &(at, extent) in items {
        min_x = min_x.min(at[0]);
        min_y = min_y.min(at[1]);
        max_x = max_x.max(at[0].wrapping_add(extent[0]));
        max_y = max_y.max(at[1].wrapping_add(extent[1]));
    }
    let side = |coordinate: Coordinate| {
        if coordinate.percent {
            0
        } else {
            coordinate.value
        }
    };
    let padding = grid.padding;
    let [left, top, right, bottom] = [
        side(padding.left),
        side(padding.top),
        side(padding.right),
        side(padding.bottom),
    ];
    let position_adjust = [
        left.min(left.wrapping_add(min_x)),
        top.min(top.wrapping_add(min_y)),
    ];
    let size_adjust = [
        right.max(right.wrapping_add(max_x.wrapping_sub(min_x).wrapping_sub(size[0]))),
        bottom.max(bottom.wrapping_add(max_y.wrapping_sub(min_y).wrapping_sub(size[1]))),
    ];
    let no_parent = |why| Refusal::new(&element.path, RefusalKind::NoParent, why);
    let origin = grid_origin(element, grid, parent, [0, 0], set).map_err(no_parent)?;
    let relaid = grid_origin(element, grid, parent, position_adjust, set).map_err(no_parent)?;
    Ok(GridLayout {
        origin,
        relaid,
        items: items.iter().map(|&(at, _)| at).collect(),
        extent: [
            size[0].wrapping_add(size_adjust[0]),
            size[1].wrapping_add(size_adjust[1]),
        ],
    })
}

fn grid_origin(
    element: &Element,
    grid: &native::Grid,
    parent: [i32; 2],
    adjust: [i32; 2],
    set: Option<[i32; 2]>,
) -> Result<[i32; 2], String> {
    if let Some(set) = set {
        return Ok([set[0] + adjust[0], set[1] + adjust[1]]);
    }
    let position = evaluate_position(
        &grid.position,
        Some(parent),
        format_args!("{}.position", element.path),
    )?;
    let [pw, ph] = parent;
    let anchor = match grid.orientation {
        1 => [pw, 0],
        2 => [0, ph],
        3 => [pw, ph],
        4 => [pw / 2, ph / 2],
        7 => [pw / 2, 0],
        8 => [pw / 2, ph],
        _ => [0, 0],
    };
    Ok([
        position[0] + adjust[0] + anchor[0],
        position[1] + adjust[1] + anchor[1],
    ])
}

impl GridLayout {
    pub fn places(&self) -> Vec<[i32; 2]> {
        self.items
            .iter()
            .map(|item| {
                [
                    self.origin[0].wrapping_add(item[0]),
                    self.origin[1].wrapping_add(item[1]),
                ]
            })
            .collect()
    }
}

impl Holder {
    #[cfg(any(test, feature = "test-support"))]
    pub fn finalize(
        &self,
        grid: &Element,
        viewport: [i32; 2],
        items: &[&Element],
    ) -> Result<Vec<[i32; 2]>, Refusal> {
        Ok(finalize_grid(grid, self.content, viewport, items, self.position)?.places())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn item_opacity(&self) -> Option<f32> {
        (self.opacity != 1.0).then_some(self.opacity)
    }

    pub fn place(&self, place: [i32; 2], item: &Layout) -> Vec<Part> {
        self.place_parts(place, &item.parts)
    }

    pub fn place_regions(&self, place: [i32; 2], regions: &[Region]) -> Vec<Region> {
        let k = self.scale;
        let at = [
            self.origin[0] + k * place[0] as f32,
            self.origin[1] + k * place[1] as f32,
        ];
        let shift = [self.logical[0] + place[0], self.logical[1] + place[1]];
        regions
            .iter()
            .map(|region| {
                let own = region.clip.map(|clip| {
                    [
                        clip[0] + shift[0],
                        clip[1] + shift[1],
                        clip[2] + shift[0],
                        clip[3] + shift[1],
                    ]
                });
                Region {
                    path: region.path.clone(),
                    owner: region.owner.clone(),
                    background_index: region.background_index,
                    scroll: region.scroll,
                    slide: region.slide,
                    rect: [
                        at[0] + k * region.rect[0],
                        at[1] + k * region.rect[1],
                        k * region.rect[2],
                        k * region.rect[3],
                    ],
                    clip: match (own, self.clip) {
                        (Some(own), holder) => intersect(holder, own),
                        (None, holder) => holder,
                    },
                    hit: region.hit.as_ref().map(|hit| {
                        hit.placed(
                            self,
                            place,
                            match (own, self.clip) {
                                (Some(own), holder) => intersect(holder, own),
                                (None, holder) => holder,
                            },
                        )
                    }),
                    disabled: region.disabled,
                }
            })
            .collect()
    }

    pub fn place_scrollbar(&self, rows: &widgets::ListBoxRows) -> Vec<Part> {
        let mut parts = self.place_parts([0, 0], &rows.scrollbar);
        if !rows.clipped() {
            for part in &mut parts {
                part.clip = None;
            }
        }
        parts
    }

    pub fn place_parts<'a>(
        &self,
        place: [i32; 2],
        parts: impl IntoIterator<Item = &'a Part>,
    ) -> Vec<Part> {
        let k = self.scale;
        let at = [
            self.origin[0] + k * place[0] as f32,
            self.origin[1] + k * place[1] as f32,
        ];
        let shift = [self.logical[0] + place[0], self.logical[1] + place[1]];
        let point = |p: [f32; 2]| [at[0] + k * p[0], at[1] + k * p[1]];
        parts
            .into_iter()
            .map(|part| {
                let mut part = part.clone();
                part.child = self.child;
                super::native_shader::draw::transform(&mut part, k, at);
                match &mut part.kind {
                    PartKind::Sprite { geometry, .. }
                    | PartKind::Cell { geometry, .. }
                    | PartKind::Progress { geometry, .. }
                    | PartKind::TextSprite { geometry, .. } => {
                        let [x, y] = point([geometry.rect[0], geometry.rect[1]]);
                        geometry.rect = [x, y, geometry.rect[2] * k, geometry.rect[3] * k];
                        geometry.pivot = point(geometry.pivot);
                    }
                    PartKind::Animated(animated) => {
                        let geometry = &mut animated.geometry;
                        let [x, y] = point([geometry.rect[0], geometry.rect[1]]);
                        geometry.rect = [x, y, geometry.rect[2] * k, geometry.rect[3] * k];
                        geometry.pivot = point(geometry.pivot);
                    }
                    PartKind::Text { glyphs, scale, .. } => {
                        for glyph in glyphs {
                            glyph.at = point(glyph.at);
                        }
                        *scale *= k;
                    }
                    PartKind::Inline { rect, .. } | PartKind::MaskedFlag { rect, .. } => {
                        let [x, y] = point([rect[0], rect[1]]);
                        *rect = [x, y, rect[2] * k, rect[3] * k];
                    }
                    PartKind::Fan { centre, radius, .. } => {
                        *centre = point(*centre);
                        *radius = radius.map(|value| value * k);
                    }
                    PartKind::Tiles { quads, .. } => {
                        for (rect, _) in quads {
                            let [x, y] = point([rect[0], rect[1]]);
                            *rect = [x, y, rect[2] * k, rect[3] * k];
                        }
                    }
                }
                let own = part.clip.map(|clip| {
                    [
                        clip[0] + shift[0],
                        clip[1] + shift[1],
                        clip[2] + shift[0],
                        clip[3] + shift[1],
                    ]
                });
                part.clip = match (own, self.clip) {
                    (Some(own), holder) => intersect(holder, own),
                    (None, holder) => holder,
                };
                part
            })
            .collect()
    }
}

pub fn draw_composed(
    layout: &Layout,
    items: &[Vec<Part>],
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
) -> Result<(), Refusal> {
    let _submit_profile = viewer_runtime::profiling::span("dc_gui_submit");
    let first = batch.quads.len();
    let mut previous = None;
    for part in layout.composed_parts(items) {
        if let Err(refusal) = draw_part(part, origin, sheets, batch, &mut previous) {
            batch.quads.truncate(first);
            return Err(refusal);
        }
    }
    Ok(())
}

fn list_origin(list: &native::List, parent: [i32; 2], set: Option<[i32; 2]>) -> [i32; 2] {
    let offset = match list.orientation {
        Orientation::UpperRight => [parent[0], 0],
        Orientation::LowerLeft => [0, parent[1]],
        Orientation::LowerRight => parent,
        Orientation::Center => [parent[0] / 2, parent[1] / 2],
        _ => [0, 0],
    };
    let position = set.unwrap_or(list.position);
    [
        position[0].wrapping_add(offset[0]),
        position[1].wrapping_add(offset[1]),
    ]
}

pub fn list_places(
    element: &Element,
    parent: [i32; 2],
    entry: [i32; 2],
    count: usize,
    set: Option<[i32; 2]>,
) -> Result<Vec<(usize, [i32; 2])>, String> {
    let list = element
        .list()
        .ok_or_else(|| format!("{} is not an OverlappingElementsBoxType", element.path))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let [w, h] = list.size.map(|value| value as f32);
    let [ew, eh] = entry.map(|value| value as f32);
    let n = count as f32;
    let spacing = list.spacing as f32;
    let fewer = (count - 1) as f32;
    let mut step_x = spacing + ew;
    if w < n * step_x - spacing {
        step_x = (w - ew) / fewer;
    }
    let mut step_y = spacing + eh;
    if h < n * step_y - spacing {
        step_y = (h - eh) / fewer;
    }
    let mut origin = list_origin(list, parent, set);
    let mut reversed = false;
    match list.format {
        ListFormat::Center => {
            origin[0] = origin[0].wrapping_add(truncate(f64::from(
                w * 0.5 - (fewer * step_x + ew + 0.5) * 0.5,
            )));
            step_y = 0.0;
        }
        ListFormat::Right => {
            step_y = 0.0;
            origin[0] = origin[0].wrapping_add(truncate(f64::from(w - ew)));
            reversed = true;
        }
        ListFormat::Left => step_y = 0.0,
        ListFormat::Up => {
            step_x = 0.0;
            origin[1] = origin[1].wrapping_add(truncate(f64::from(h - eh)));
            reversed = true;
        }
        ListFormat::Down => step_x = 0.0,
    }
    let order: Vec<usize> = if reversed {
        (0..count).rev().collect()
    } else {
        (0..count).collect()
    };
    let mut places = vec![[0; 2]; count];
    for (step, &entry) in order.iter().enumerate() {
        let i = step as f32;
        places[entry] = [
            truncate(f64::from(i * step_x) + 0.5).wrapping_add(origin[0]),
            truncate(f64::from(i * step_y) + 0.5).wrapping_add(origin[1]),
        ];
    }
    let mut painted: Vec<(usize, [i32; 2])> = places.into_iter().enumerate().collect();
    if list.first_on_top {
        painted.reverse();
    }
    Ok(painted)
}

fn truncate(value: f64) -> i32 {
    if value.is_nan() || value <= f64::from(i32::MIN) - 1.0 || value >= f64::from(i32::MAX) + 1.0 {
        i32::MIN
    } else {
        value as i32
    }
}

pub trait Sheets {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]>;
    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Kept {
    technique: u32,
    samplers: [Option<u32>; 3],
    lod_bias: [f32; 3],
}

pub fn draw(
    layout: &Layout,
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
) -> Result<(), Refusal> {
    let _submit_profile = viewer_runtime::profiling::span("dc_gui_submit");
    let first = batch.quads.len();
    let mut previous = None;
    let drawn = layout
        .parts
        .iter()
        .try_for_each(|part| draw_part(part, origin, sheets, batch, &mut previous));
    if drawn.is_err() {
        batch.quads.truncate(first);
    }
    drawn
}

fn draw_part(
    part: &Part,
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
    previous: &mut Option<Kept>,
) -> Result<(), Refusal> {
    batch.at_part(&part.path);
    if part.shading.compiled.is_some() {
        return super::native_shader::draw::emit(part, origin, sheets, batch);
    }
    #[cfg(any(test, feature = "test-support"))]
    return draw_fixture_part(part, origin, sheets, batch, previous);
    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = previous;
        Err(Refusal::new(
            &part.path,
            RefusalKind::CustomEffect,
            "effect has no compiled technique asset",
        ))
    }
}

#[cfg(any(test, feature = "test-support"))]
fn draw_fixture_part(
    part: &Part,
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
    previous: &mut Option<Kept>,
) -> Result<(), Refusal> {
    let [ox, oy] = origin.map(|value| value as f32);
    let scissor = part.clip.map(|clip| {
        [
            clip[0] + origin[0],
            clip[1] + origin[1],
            clip[2] + origin[0],
            clip[3] + origin[1],
        ]
    });
    let before = |what: &str| {
        Refusal::new(
            &part.path,
            RefusalKind::MissingInput,
            format!(
                "{what}, which keeps what the game's previous draw bound, and that draw is not \
                 this window's: the state is outside the recording"
            ),
        )
    };
    let (blend, bound) = match part.shading.fixture {
        Some((fixture, blend, samplers)) => {
            let mut bound = Kept {
                technique: fixture.id,
                samplers: [None; 3],
                lod_bias: [0.0; 3],
            };
            for (index, slot) in [0, 1, 5].into_iter().enumerate() {
                match samplers[slot] {
                    Some(sampler) => {
                        bound.samplers[index] = Some(sampler.packed());
                        bound.lod_bias[index] = sampler.lod_bias;
                    }
                    None => {
                        let kept = previous.and_then(|previous| {
                            Some((previous.samplers[index]?, previous.lod_bias[index]))
                        });
                        match kept {
                            Some((sampler, bias)) => {
                                bound.samplers[index] = Some(sampler);
                                bound.lod_bias[index] = bias;
                            }
                            None if index == 0 => {
                                return Err(before("its effect declares no sampler for slot 0"));
                            }
                            None => {}
                        }
                    }
                }
            }
            (blend, bound)
        }
        None => {
            if matches!(part.kind, PartKind::Sprite { masked: true, .. }) {
                return Err(Refusal::new(
                    &part.path,
                    RefusalKind::NullRead,
                    "its effect lacks the technique it draws with, and with its masking texture \
                     bound the binder reads through the null technique record: the game crashes",
                ));
            }
            let kept = previous.ok_or_else(|| {
                before(
                    "its effect lacks the technique it draws with, so the null technique record \
                     keeps the program and the slot samplers",
                )
            })?;
            (Blend::Off, kept)
        }
    };
    *previous = Some(bound);
    let (technique, lod_bias) = (bound.technique, bound.lod_bias);
    let samplers = bound
        .samplers
        .iter()
        .enumerate()
        .fold(0, |packed, (index, slot)| {
            packed | slot.unwrap_or(0) << (10 * index)
        });
    let exact = |corners: [[f32; 2]; 4], uv: [[f32; 2]; 4]| Exact {
        native: None,
        corners: corners.map(|[x, y]| [x + ox, y + oy]),
        uv,
        scissor,
        technique,
        blend,
        samplers,
        lod_bias,
        second: None,
        masking: None,
        offsets: [0.0; 4],
        inputs: [part.shading.since, part.shading.animation, 0.0, 0.0],
        second_color: [0.0; 4],
    };
    let corners_uv = |[u0, v0, u1, v1]: [f32; 4]| [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    let project = |corners: [[f32; 2]; 4], [fx, fy]: [f32; 2]| {
        corners.map(|[x, y]| [(x + ox) * fx - ox, (y + oy) * fy - oy])
    };
    let quad = |texture: usize, color: [f32; 4], exact: Exact| Quad {
        texture,
        rect: [0.0; 4],
        uv: [0.0; 4],
        color,
        rotation: None,
        layer: 0,
        shade: 0.0,
        exact: Some(exact),
        anchor: None,
        present: None,
    };
    let missing = |what: String| Refusal::new(&part.path, RefusalKind::MissingResource, what);
    match &part.kind {
        PartKind::Animated(_) | PartKind::TextSprite { .. } => {
            return Err(Refusal::new(
                &part.path,
                RefusalKind::CustomEffect,
                "this layout fixture has no compiled effect asset",
            ));
        }
        PartKind::Sprite {
            sprite,
            geometry,
            uv,
            offsets,
            masked,
        } => {
            let sheet = sheets
                .sheet(sprite)
                .ok_or_else(|| missing(format!("sprite {} is not loaded", lossy(sprite))))?;
            let masking = if *masked {
                Some(
                    sheets
                        .masking(sprite)
                        .ok_or_else(|| {
                            missing(format!(
                                "sprite {}'s masking texture is not loaded",
                                lossy(sprite)
                            ))
                        })?
                        .texture,
                )
            } else {
                None
            };
            let exact = Exact {
                offsets: *offsets,
                masking,
                ..exact(geometry.corners(), corners_uv(*uv))
            };
            batch.push(quad(sheet.texture, part.color, exact));
        }
        PartKind::Cell {
            texture,
            masking,
            uv,
            geometry,
        } => {
            let mut exact = exact(geometry.corners(), corners_uv(*uv));
            if let Some(sprite) = masking {
                exact.masking = Some(
                    sheets
                        .masking(sprite)
                        .ok_or_else(|| {
                            missing(format!(
                                "sprite {}'s masking texture is not loaded",
                                lossy(sprite)
                            ))
                        })?
                        .texture,
                );
            }
            batch.push(quad(*texture, part.color, exact));
        }
        PartKind::Progress {
            sprite,
            geometry,
            uv,
            state,
            colors,
            textured,
        } => {
            let (texture, second) = if *textured {
                let [one, two] = sheets.progress(sprite).ok_or_else(|| {
                    missing(format!(
                        "progress bar {}'s textures are not loaded",
                        lossy(sprite)
                    ))
                })?;
                (one.texture, Some(two.texture))
            } else {
                (super::sprites::NO_TEXTURE, None)
            };
            let exact = Exact {
                second,
                inputs: [0.0, 0.0, *state, 0.0],
                second_color: colors[1],
                ..exact(geometry.corners(), *uv)
            };
            batch.push(quad(texture, colors[0], exact));
        }
        PartKind::Tiles { sprite, quads, .. } => {
            let sheet = sheets
                .sheet(sprite)
                .ok_or_else(|| missing(format!("sprite {} is not loaded", lossy(sprite))))?;
            for ([x, y, w, h], uv) in quads {
                let exact = exact(
                    [[*x, *y], [x + w, *y], [x + w, y + h], [*x, y + h]],
                    corners_uv(*uv),
                );
                batch.push(quad(sheet.texture, part.color, exact));
            }
        }
        PartKind::MaskedFlag {
            sprite,
            flag,
            flag_coords,
            rect,
            ..
        } => {
            let mask = sheets.mask(sprite).ok_or_else(|| {
                missing(format!(
                    "masked shield {}'s mask is not loaded",
                    lossy(sprite)
                ))
            })?;
            let [x, y, w, h] = *rect;
            let exact = Exact {
                second: Some(mask.texture),
                offsets: *flag_coords,
                ..exact(
                    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                    corners_uv([0.0, 0.0, 1.0, 1.0]),
                )
            };
            batch.push(quad(*flag, part.color, exact));
        }
        PartKind::Inline {
            source,
            rect,
            uv,
            modulate,
            projection,
            ..
        } => {
            let texture = match source {
                InlineSource::Sprite(sprite) => {
                    sheets
                        .sheet(sprite)
                        .ok_or_else(|| missing(format!("sprite {} is not loaded", lossy(sprite))))?
                        .texture
                }
                InlineSource::Texture(texture) => *texture,
            };
            let [x, y, w, h] = *rect;
            let exact = Exact {
                second_color: *modulate,
                ..exact(
                    project(
                        [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                        *projection,
                    ),
                    corners_uv(*uv),
                )
            };
            batch.push(quad(texture, part.color, exact));
        }
        PartKind::Fan {
            sprite,
            centre,
            radius,
            rotation,
            weights,
        } => {
            let [one, two] = sheets.progress(sprite).ok_or_else(|| {
                missing(format!(
                    "circular progress bar {}'s textures are not loaded",
                    lossy(sprite)
                ))
            })?;
            let rim = |angle: f32| {
                (
                    [
                        centre[0] + angle.cos() * radius[0],
                        centre[1] - angle.sin() * radius[1],
                    ],
                    [angle.cos(), angle.sin()],
                )
            };
            let mut previous = rim(*rotation);
            let mut k = 0i32;
            for (sector, weight) in weights.iter().enumerate() {
                for _ in 0..(*weight).max(0) {
                    k += 1;
                    let next = rim((k as f32 * 6.283_185_5) / 100.0 + rotation);
                    let exact = Exact {
                        second: Some(two.texture),
                        inputs: [0.0, 0.0, sector as f32, 0.0],
                        ..exact(
                            [previous.0, *centre, next.0, next.0],
                            [previous.1, [0.0, 0.0], next.1, next.1],
                        )
                    };
                    batch.push(quad(one.texture, part.color, exact));
                    previous = next;
                }
            }
        }
        PartKind::Text {
            glyphs,
            scale,
            modulate,
            projection,
            ..
        } => {
            for glyph in glyphs {
                let [x, y] = glyph.at;
                let [w, h] = glyph.size.map(|value| value as f32 * scale);
                let corners = project(
                    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                    *projection,
                );
                let exact = Exact {
                    second_color: *modulate,
                    ..exact(corners, corners_uv(glyph.uv))
                };
                batch.push(quad(glyph.texture, glyph.color, exact));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
