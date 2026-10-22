//! Lays out and draws [`installed_content_core::gui::native`] element trees with the retained state owned
//! by [`lifecycle::Instance`]. Windows preserve attach order, evaluated position and size,
//! anchors and margins. Controls preserve authored or code-written sprites, frames, text,
//! progress, scale and state. Grids and overlapping lists use [`grid_slots`] and
//! [`list_places`] and lay each item out through its own instance.
//!
//! Geometry remains in floating interface pixels through the composed transform and
//! rasterization. Whole-pixel clips become scissors. Parts retain effect techniques and
//! sampler state; production [`draw`] submits compiled effects to [`Batch`]. Test fixtures
//! retain geometry and technique metadata in [`super::sprites::Exact`].
//!
//! Unavailable inputs or unsupported game behavior refuse the complete window with counted
//! [`Refusal`] reasons. Cases that draw nothing without failing produce counted [`Diagnostic`]
//! entries.

#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap};
use std::{borrow::Cow, cell::RefCell};

use installed_content_core::{
    gui::native, gui::native::Animation, gui::native::Coordinate, gui::native::Element,
    gui::native::Fields, gui::native::GridFormat, gui::native::Kind, gui::native::ListFormat,
    gui::native::Orientation, gui::native::TextFormat, gui::native::VerticalAlignment,
    gui::native::fonts::TextColors,
};
use installed_content_localisation::localisation::Localisation;

pub mod animation;
pub mod fixed;
mod hit;
pub mod lifecycle;
pub mod text_sprites;
pub mod widgets;

pub use animation::{Animated, AnimationRecord};
pub use fixed::fixed_window_layout;
pub use hit::{AlphaMask, HitGeometry};

use super::{
    native_effects::{Blend, Effect, FixtureTechnique, Sampler, Settles, Technique},
    native_text::{self, Limits, Piece, Placement},
    sprites::{Batch, SpriteSheet},
};

#[cfg(any(test, feature = "test-support"))]
use super::sprites::{Exact, Quad};

/// An identifier's bytes as a diagnostic shows them. Identifiers themselves (sprite, font,
/// Texture, template and effect names) stay the reader's raw bytes end to end, compared byte
/// for byte as the game compares them; only a message's text is lossy.
pub fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The fallback sprite for an empty or unknown sprite name.
pub const FALLBACK_SPRITE: &[u8] = installed_content_core::gui::native::resources::FALLBACK_SPRITE;

/// Why a window cannot be drawn as the game draws it, for one element: its kind names the reason
/// for counting ([`Refusal::kind`]), its detail says what the game does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The refused element's path.
    pub path: String,
    pub kind: RefusalKind,
    pub detail: String,
}

/// A refusal's reason, which callers count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefusalKind {
    /// The GUI reader refused a key of the element ([`native::Refusal`]).
    Reader,
    /// The game reads memory it never wrote (an uninitialised pointer or record).
    UndefinedRead,
    /// The game reads through a null pointer.
    NullRead,
    /// The game treats an object as a kind it is not: it reads fields past it or of another
    /// layout, or calls a vtable slot that holds no function.
    TypeConfusion,
    /// The game divides an integer by zero, which ends it.
    DivideByZero,
    /// The game never leaves a loop, such as a cornered tile's pulse wrap.
    Hang,
    /// A value the viewer's recording lacks, which the game's code supplies at run time.
    MissingInput,
    /// An effect that fails to parse, compile or validate, with the effect and compiler reason.
    CustomEffect,
    /// A sprite, texture or font the caller has not loaded.
    MissingResource,
    /// A size or position that needs the parent's extent, which the caller did not give.
    NoParent,
}

impl RefusalKind {
    /// The name a caller counts it under.
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

/// Every refusal of one window.
pub type Refusals = Vec<Refusal>;

/// What the game draws nothing for without failing, per element: counted by kind, not refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub kind: DiagnosticKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticKind {
    /// Neither the sprite nor [`FALLBACK_SPRITE`] is defined, so no drawable is made.
    MissingFallback,
    /// A text names an inline icon the content does not define: finds nothing, and the icon
    /// takes no width and draws nothing.
    UndefinedTextIcon,
    /// An inline flag has no valid resident atlas record.
    NullTextFlag,
    /// A text sprite produces no render target.
    EmptyTextBake,
    /// A sprite whose missing texture prevents drawing.
    NoTexture,
    /// A container names a scrollbar with no `extendedScrollbarType` template, so it has none.
    MissingScrollbar,
    /// The text bytes fail UTF-8 conversion ([`native_text::decode`]), so the game draws
    /// nothing.
    UndecodableText,
    /// A glyph on a font page whose texture does not load ([`native_text::GlyphQuad::null_page`]):
    /// It takes its place and draws with the null texture.
    NullFontPage,
    /// A map icon pool was exhausted and grew ([`lifecycle::Pool::take_diagnostics`]): the
    /// frame the game's worker task completes the growth on is not in the recording.
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

/// Counts of refusals and diagnostics by name, which a caller adds to its report.
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

/// What the layout needs to know of a sprite from its `.gfx` definition and its loaded texture.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteInfo {
    /// The texture size in texels, divided by the frame grid.
    pub texture: [i32; 2],
    /// `noOfFrames` as stored (`sprite type`), unchecked.
    pub frames: i32,
    /// `framedirectiony`: the frames run down the texture.
    pub vertical: bool,
    /// Whether `masking_texture` loads: enable the `MASKING` effect variant and bind the mask
    /// to slot 5 ([`Sheets::masking`]).
    pub masked: bool,
    /// Whether it draws its `animation` records: loaded eagerly (`legacy_lazy_load = no`) with
    /// a record whose type is not 0, which compiles its effect with `ANIMATED` and runs on the
    /// clock.
    pub animated: bool,
    pub kind: SpriteKind,
    /// `anchor` (`sprite type`), which `center_on_anchor` subtracts.
    pub anchor: [f32; 2],
    /// The sprite's resolved effect, or why it is refused
    /// ([`super::native_effects::Effect::load`]).
    pub effect: Result<Effect, String>,
    /// How the pointer is tested against it.
    pub hit: Hit,
    /// When false, plain, tile, frame-animated and masked-shield sprites draw nothing and
    /// report zero extent.
    pub missing: Option<String>,
    /// Why the sprite is not taken as the game takes it, by kind: the reader's refusals of its
    /// declaration ([`installed_content_core::gui::native::sprites::SpriteFacts::refusals`],
    /// [`RefusalKind::Reader`]), a texture whose bytes leave the draw undetermined
    /// ([`RefusalKind::UndefinedRead`]; [`RefusalKind::MissingInput`] for the device) or at a
    /// path the layers cannot address ([`RefusalKind::MissingResource`]). A layout that draws or measures it is refused by
    /// these reasons.
    pub refusals: Vec<(RefusalKind, String)>,
}

/// Truncates f32 to i32 using the shared integer-conversion implementation
/// ([`installed_content_core::crt::truncate_f32`]).
use installed_content_core::crt::truncate_f32;

impl SpriteInfo {
    /// Frame grid dimensions: `noOfFrames` horizontally and one vertically, with the axes
    /// swapped by `framedirectiony`.
    fn divisors(&self) -> [i32; 2] {
        if self.vertical {
            [1, self.frames]
        } else {
            [self.frames, 1]
        }
    }

    /// Multiplies the reciprocal frame count by the texture size on each axis, then
    /// truncates to the integer extent (a 10-texel strip of 3 frames is 3 wide).
    pub fn extent(&self) -> [i32; 2] {
        let divisors = self.divisors();
        [0, 1]
            .map(|axis| truncate_f32((1.0f32 / divisors[axis] as f32) * self.texture[axis] as f32))
    }

    /// The floating frame size used for drawing, retaining fractional texels.
    pub fn frame_size(&self) -> [f32; 2] {
        let divisors = self.divisors();
        [0, 1].map(|axis| (1.0f32 / divisors[axis] as f32) * self.texture[axis] as f32)
    }

    /// The frame's texture-coordinate extent and step. Each extent multiplies the texture
    /// size by its reciprocal, then by the reciprocal frame count; the step is the extent
    /// along the strip and zero across it.
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

    /// The drawable's queried extent, scaled and truncated, except progress bars retain their
    /// unscaled type size.
    pub fn scaled_extent(&self, scale: f32) -> [i32; 2] {
        match self.kind {
            SpriteKind::Progress { size, .. } => size,
            // Its size, unscaled.
            SpriteKind::CorneredTile { size, .. } => size,
            // The overlay's size.
            SpriteKind::MaskedShield { .. } => self.texture,
            // Size overrides apply on both axes.
            SpriteKind::PieChart { size } => [size.unwrap_or(0); 2],
            // The embedded pie's size, unscaled.
            SpriteKind::CircularProgress { size, .. } => [size; 2],
            SpriteKind::LineChart { size } => size,
            _ => self
                .extent()
                .map(|value| truncate_f32(value as f32 * scale)),
        }
    }
}

/// How the pointer is tested inside a sprite frame, including lower edges and excluding upper
/// edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hit {
    /// The frame rectangle.
    #[default]
    Frame,
    /// Passed through: the sprite's own `alwaystransparent`, or no texture loaded.
    Through,
    /// `transparencecheck`: the frame's alpha under the pointer, which needs its texels.
    Alpha,
}

/// A sprite kind by how it draws.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteKind {
    /// `spriteType` or `quadTextureSprite` (`sprite type`).
    Plain,
    /// A `progressbartype` drawable.
    Progress {
        /// The unscaled extent spanned by the quad.
        size: [i32; 2],
        /// `steps`, initially 100: the divisor and upper clamp of the value.
        steps: i32,
        /// `color` and `colorTwo`, initially white.
        colors: [[f32; 4]; 2],
        /// `horizontal`, initially true.
        horizontal: bool,
        /// Whether `textureFile1` and `textureFile2` are both set: resource preparation registers
        /// each only when set, and looks up `Texture` only when both are, `Color` otherwise.
        textured: bool,
    },
    /// A `corneredTileSpriteType` nine-slice at the drawable size. `borderSize` preserves
    /// corners, `tilingCenter` repeats the centre, and `looping` pulses alpha.
    CorneredTile {
        size: [i32; 2],
        border: [i32; 2],
        tiling_center: bool,
        looping: bool,
        /// The pulse's rate (`animation_rate_fps` as read, `animation_rate_spf`
        /// inverted), `None` while unwritten.
        rate: Option<f32>,
    },
    /// `tileSpriteType`: its texture repeated over `size`.
    Tile { size: [i32; 2] },
    /// A `resizeableSpriteType` copies its size into the type, then invokes a draw slot that
    /// emits nothing.
    Resizeable,
    /// `frameAnimatedSpriteType`: a plain sprite whose frame a clock advances (its instance's
    /// clock, [`lifecycle::Instance`]).
    FrameAnimated {
        /// Seconds per frame: the reciprocal of `animation_rate_fps`, or `animation_rate_spf`
        /// directly, initially 1.
        seconds_per_frame: f32,
        looping: bool,
        play_on_show: bool,
        /// `pause_on_loop`, seconds.
        pause: f32,
    },
    /// `maskedShieldType`: a flag cut by its mask (`textureFile2`) under its overlay
    /// (`textureFile1`, the sprite's `texture`), the small flag atlas under `small`.
    MaskedShield {
        /// The mask texture's size.
        mask: [i32; 2],
        small: bool,
    },
    /// A `PieChartType` with an uninitialized size (`None`) refuses when the factory measures
    /// it. The constructor supplies no slices, so it otherwise draws nothing.
    PieChart { size: Option<i32> },
    /// `LineChartType`: its lines are the code's, none at construction, so it draws nothing.
    LineChart { size: [i32; 2] },
    /// A `circularProgressBarType` fan with radius `size` about the drawable origin, starting
    /// at `rotation` radians. Its sectors represent remaining and completed shares of 100.
    CircularProgress {
        size: i32,
        rotation: f32,
        amount: f32,
    },
    /// `textSpriteType` (`text sprite type`): a text baked into a render target
    /// ([`text_sprites`]); its `noOfFrames` is [`SpriteInfo::frames`] (a `%u` as stored), its
    /// `textureFile` the sprite's texture, which `textured` says is named.
    Text { textured: bool },
}

/// The sprites and fonts a layout draws, as the caller has them.
pub trait Resources {
    /// The frame's physical extent (graphics state), separate from the logical construction
    /// viewport. Required only by native pointer geometry.
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        None
    }
    /// The sprite the content defines under `name`, if any.
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo>;

    /// Borrow immutable loaded metadata when the resource store retains it.
    fn sprite_info(&self, name: &[u8]) -> Option<std::borrow::Cow<'_, SpriteInfo>> {
        self.sprite(name).map(std::borrow::Cow::Owned)
    }
    /// A recorded portrait sprite resolved to its texture, without replacing the icon's mask.
    fn portrait_texture(&self, _name: &[u8]) -> Option<Texture> {
        None
    }

    /// The CPU transparency mask, when the texture loader made one. No mask accepts
    /// the rectangular hit; a missing texture is separately Hit::Through.
    fn sprite_alpha(&self, _name: &[u8]) -> Option<AlphaMask> {
        None
    }
    /// A declared sprite whose resources the caller has not loaded, distinct from an undefined name.
    fn unloaded_sprite(&self, _name: &[u8]) -> bool {
        false
    }
    /// The font resolves `name` to, with the glyphs of the family the viewer's language selects
    /// ([`native_text::Font`]), `None` when the registry holds no font at all.
    fn font(&self, name: &[u8]) -> Option<&native_text::Font>;
    /// The content stack's localisation in the viewer's language, which an authored text key
    /// is shown through.
    fn localisation(&self) -> &Localisation;
    /// The global `textcolors` table: the last block the content stack's `.gfx` files declare.
    fn text_colors(&self) -> &TextColors;
    /// The content-resolved effect every bitmap font draws with.
    fn text_effect(&self) -> Result<Effect, String>;
    /// The effect file `file` resolves to through the content stack, for the paths the game's
    /// code names itself (a masked shield's overlay draws with `gfx/FX/buttonstate.shader`'s
    /// `Up` technique).
    fn effect(&self, file: &str) -> Result<Effect, String>;
    /// A masked shield's flag atlas cell at `index`, using the small or large atlas.
    /// Out-of-range indices select record 0. Returns coordinates with the cell; `Ok(None)`
    /// means the record or its page is unavailable, so only the overlay draws.
    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String>;
    /// The small flag selected by a text's `0x12` tag sequence. Unknown tags produce index -1,
    /// out-of-range tag and atlas lookups fall back to entry 0, and invalid or unloaded cells
    /// draw nothing. Errors name missing recorded inputs.
    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String>;
    /// Producer text already chose this full flag-atlas key from recorded country candidates.
    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
        Err(format!(
            "resolved_text_flag: atlas key {key:?} has no bound flag atlas"
        ))
    }
    /// Longest Normal-atlas key starting this producer-bound text, in codepoints.
    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
        let _ = text;
        None
    }
    /// Over the one registry every `.gui` file's `guiTypes` insert into
    /// ([`installed_content_core::gui::native::Guis::template`]): the scrollbars a container, a list box or
    /// a scrollbar's parts name are found there.
    fn gui_type(&self, name: &[u8]) -> Option<&Element>;
    /// A texture file the game's code loads by path (an edit box's `textureFile` its text
    /// sprite bakes over, [`Assets::textures`]): `Some(Err)` naming why it does not load,
    /// `None` when the caller has not loaded it (which is refused by name).
    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        let _ = path;
        None
    }
    /// The animation records an eagerly loaded sprite draws ([`SpriteInfo::animated`]), as
    /// loaded; `None` when the caller has not loaded them (which is refused by name).
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        let _ = sprite;
        None
    }
    /// Whether the content names a `textBoxType` whose constructor can register a session text
    /// sprite ([`installed_content_core::gui::native::Guis::text_box_named`]). Such a sprite persists under
    /// the template name after construction, so later absent-sprite lookup can depend on
    /// unrecorded session history. The default `false` states that no such template exists.
    fn session_text_sprite(&self, name: &[u8]) -> bool {
        let _ = name;
        false
    }
}

/// Looks up a sprite through [`Resources::sprite`] and counts its refusals ([`noted`]). A name
/// matching a text-box template can depend on an unrecorded session sprite and is refused
/// ([`Resources::session_text_sprite`]). A text box can create its own absent name without
/// session history.
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

/// A reader refusal ([`native::Refusal`]) as the layout counts it: a value of the recording
/// client's session the recording does not hold ([`native::Cause::Unrecorded`]) is missing
/// input; anything else the reader refused.
pub(super) fn reader_refusal(refusal: &native::Refusal) -> Refusal {
    let kind = match refusal.cause {
        native::Cause::Unrecorded => RefusalKind::MissingInput,
        _ => RefusalKind::Reader,
    };
    Refusal::new(&refusal.path, kind, refusal.to_string())
}

/// The reader's refusals of `element` and every element it holds, as the layout counts them.
pub(super) fn reader_refusals(element: &Element) -> Refusals {
    element
        .subtree_refusals()
        .into_iter()
        .map(reader_refusal)
        .collect()
}

/// Resolves authored localisation bytes with `$KEY$` substitutions and stored control codes
/// ([`Localisation::localise_bytes`]). An undefined key retains its own bytes; an empty key
/// supplies nothing. Buttons pass those bytes directly to text decoding, without scripted-text
/// evaluation, so brackets remain glyphs.
fn localised(resources: &dyn Resources, key: &[u8]) -> Option<String> {
    Some(
        shown_chars(resources.localisation(), key)?
            .into_iter()
            .collect(),
    )
}

/// [`localised`]'s codepoints: the one path from an authored key to what its text object draws,
/// which asset collection reads the inline icons of too ([`Assets::add_localised`]).
fn shown_chars(localisation: &Localisation, key: &[u8]) -> Option<Vec<char>> {
    native_text::decode(&localisation.localise_bytes(key, &[]).ok()?)
}

/// Which of the game's sprite setters the code replaced an element's sprite through; each
/// rebuilds the drawable and restores its own set of properties. A name the sprite registry does
/// not define changes nothing (each returns before touching the drawable), and neither does the
/// name the element already shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteSetter {
    /// After the replacement it sets the authored scale and rotation again and places the
    /// drawable as construction does (orientation, centring on the scaled extent), as if the
    /// sprite were authored.
    #[default]
    Button,
    /// The frame `max(frame, 1)` (the state no longer picks it), the raw position with no
    /// orientation offset, centred on the unscaled integer extent before the scale is set, and no
    /// rotation.
    ButtonStandard,
    /// The new drawable's own frame (-1, the state's), the scale, placed as construction places
    /// it, and no rotation.
    Icon,
}

/// What the game's code sets on one element at run time, and (private, written only by its
/// [`lifecycle::Instance`]) what the element's runtime keeps: its visibility flags, a window's
/// opacity and position, its drawable's clocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    /// Sprite replacement through [`Bound::setter`]: outside an instance, one call on the
    /// authored drawable; on an instance's control ([`lifecycle::Instance::control`]), its next
    /// call after every call made before ([`lifecycle::Instance::set_sprite`]).
    pub sprite: Option<Vec<u8>>,
    pub setter: SpriteSetter,
    /// The assigned frame, one-based as the element's own; on a progress bar's drawable, its value.
    pub frame: Option<i32>,
    /// The text a text box's code sets, as the game's text layout receives it (a localised text
    /// through [`Localisation::localise`]).
    pub text: Option<String>,
    /// Producer text carries viewer-resolved flag keys; authored text keeps native codes.
    pub resolved_flags: bool,
    /// The element's `element` flags as its instance keeps them; `None` in a layout with no
    /// lifecycle, which takes every element as its code shows it ([`lifecycle::Shown`]).
    shown: Option<lifecycle::Shown>,
    /// A straight RGBA tint.
    pub color: Option<[f32; 4]>,
    /// A progress bar's value (stored as given, clamped above to `steps` only).
    pub progress: Option<i32>,
    /// Raw value written by the normalized float setter, after its own clamp,
    /// double scaling and float rounding. Rendering does not apply the integer
    /// setter's upper clamp again.
    pub progress_float: Option<i32>,
    /// A texture the code draws in the element's place instead of a sprite's frame (a flag
    /// atlas cell); its size replaces the sprite's frame in the element's geometry.
    pub texture: Option<Texture>,
    /// The drawable's vertical stretch, with horizontal stretch fixed at 1, applied to the quad's
    /// height about its centre, the texture squeezed with it (a counter bar's fill).
    pub stretch: Option<f32>,
    /// How far the code moves the element from the place it has (`set_position` of that place
    /// plus the offset: moves a counter bar by its fill), before it is centred.
    pub offset: Option<[i32; 2]>,
    /// The code-written rotation in radians replaces authored rotation. A flipped naval-combat
    /// bar also moves by its drawable width and negative height; the caller binds that offset
    /// in [`Bound::position`].
    pub rotation: Option<f32>,
    /// Code-written drawable scale.
    pub scale: Option<f32>,
    /// A window's opacity as its instance keeps it: every sprite part of it and of
    /// its children is drawn with its alpha times this, and every text's `ModColor` alpha is
    /// set to it.
    opacity: Option<f32>,
    /// Wrapper extents from construction or the last layout update.
    window_frame: Option<Frame>,
    /// Update_layout flag zero retains its children's constructed geometry.
    layout_parent: Option<[i32; 2]>,
    /// The retained window top-left in its parent's content, without orientation or origo.
    /// Code-written grid/list positions pass through [`Holder::position`] to [`grid_slots`],
    /// [`finalize_grid`] and [`list_places`] in place of authored positions.
    pub position: Option<[i32; 2]>,
    /// The drawable its instance keeps ([`lifecycle::Instance::set_sprite`]): the one the last
    /// successful sprite replacement made, a failed one keeping it. [`Drawn::Written`] outside an
    /// Instance, where [`Bound::sprite`] is one setter call on the authored drawable.
    drawn: Drawn,
    /// A frame-animated sprite's clock: the frame its updater last wrote (`None` while the
    /// frame is the one its control or code set), and the elapsed time.
    clock: Option<(Option<i32>, f32)>,
    /// A looping cornered tile's alpha.
    pulse: Option<f32>,
    /// Why its drawable's clock hangs the game: a looping cornered tile's pulse or a
    /// frame-animated sprite's wrap.
    hang: Option<String>,
    /// A masked shield's flag index (the index itself, or `countryCount - index - 1` below
    /// zero, which the caller resolves); the constructor's 0 when unbound.
    pub shield: Option<i32>,
    /// The code-written checked state.
    pub checked: Option<bool>,
    /// The code-written enabled state.
    pub enabled: Option<bool>,
    /// An `instantTextBoxType`'s scrollbar as its instance's text writes left it: its range,
    /// value and limits, and the box's scrollbar-hidden flag, [`Walk::text_refresh`].
    text_bar: Option<(widgets::PlainState, bool)>,
    /// The last text-scroll notification; range writes alone do not move glyphs.
    text_scroll: Option<f32>,
    /// Retained content offsets in pixels. `None` preserves the construction offset.
    scroll: [Option<i32>; 2],
    /// A grid's finalize adjustments, read by its extent and overflow getters.
    grid: Option<GridBounds>,
    /// A container's retained scrollbar ranges, targets and flags.
    bars: [Option<widgets::ContainerBar>; 2],
    /// The first-draw presentation timestamp; the clock continues while hidden.
    animation_time: Option<f32>,
    /// Seconds since the drawable's state effect clock restarted: at the first draw after its
    /// state changed, however the state changed.
    effect_since: Option<f32>,
    /// Each animation record's trigger start, seconds after the first draw, with -1 meaning
    /// not started.
    triggers: Option<Vec<f32>>,
}

/// A texture file as the caller loaded it ([`Resources::texture_file`]).
#[derive(Clone, Debug, PartialEq)]
pub enum TextureFile {
    /// Its whole rectangle.
    Loaded(Texture),
    /// The game loads nothing there (the null texture), and why.
    Failed(String),
    /// A form this port has not implemented, or a path it cannot address: refused by this kind.
    Refused(RefusalKind, String),
}

/// A texture rectangle the caller has loaded: its page, its normalised `[u0, v0, u1, v1]` and
/// its size in texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texture {
    pub texture: usize,
    pub uv: [f32; 4],
    pub size: [i32; 2],
}

/// The drawable a sprite control shows, as its instance keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
enum Drawn {
    /// No instance keeps it: [`Bound::sprite`] is one setter call on the authored drawable.
    #[default]
    Written,
    /// Its instance's: the authored drawable (`None`), or the one a setter made and how.
    Kept(Option<(std::sync::Arc<[u8]>, SpriteSetter)>),
}

/// The drawable `element` shows through `bound`, when a setter made it: its sprite and the
/// setter that made it. Outside an instance, [`Bound::sprite`] replaces the authored drawable
/// when the registry defines it and it is not the one already shown
/// ([`setter_replaces`]); an instance's is the one it keeps. Drawing and every extent query
/// read the drawable through this.
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

/// Whether a sprite setter replaces the drawable showing `current`. Unknown or unchanged names
/// preserve it. A name also held by a text-box template counts as found but may depend on an
/// unrecorded session sprite, which [`looked_up`] refuses
/// ([`Resources::session_text_sprite`]).
pub(super) fn setter_replaces(current: &[u8], name: &[u8], resources: &dyn Resources) -> bool {
    name != current && sprite_resolves(name, resources)
}

/// Icon replacement also resolves session text sprites.
pub fn sprite_resolves(name: &[u8], resources: &dyn Resources) -> bool {
    resources.sprite_info(name).is_some() || resources.session_text_sprite(name)
}

/// A construction step the game takes from outside a window's own template: the object it builds
/// for its owner element from the registry or from its parent template (a scrollbar's inherited
/// parts, a container's own scrollbars, a text or list box's scrollbar, a `windowType`'s inherited
/// background), each a separate object with its own drawable, whatever template element it is
/// built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Role {
    /// A plain scrollbar's part in its constructor's order: the right and left buttons, the
    /// track, the slider, the minimum and maximum range icons.
    Part(u8),
    /// The `scrollbarType` a text box or list box builds.
    Scrollbar,
    /// A container's own `extendedScrollbarType`, vertical or not, and its parts.
    Bar(bool),
    /// A `windowType`'s background taken from its `parent` template.
    Background,
}

/// The owner element and the step that built an object from another template.
pub(super) type Via = (*const Element, Role);

/// Where an object was built: its element, and the step that built it from outside the laid-out
/// template, if any. Its runtime state is bound to it ([`Bindings`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Site {
    element: *const Element,
    via: Option<Via>,
}

impl Site {
    pub(super) fn new(element: &Element, via: Option<Via>) -> Self {
        Self {
            element: element as *const Element,
            via,
        }
    }
}

/// The run-time state of the objects a layout binds, by where each was built, and the viewer's
/// pointer.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    bound: HashMap<Site, Bound>,
    /// Collect geometry for callers that inspect controls or hovers after drawing.
    pub regions: bool,
    /// Absolute GUI clock supplied as `Time`.
    pub time: f32,
    /// Map icons draw their root without a hit list, so their caller leaves this `None`.
    pub pointer: Option<Pointer>,
    /// The GUI viewport (`graphics state`, the device size over the interface scale), which a
    /// list box's orientation and a drop-down's position are evaluated against at construction;
    /// `None` when the caller does not give it (a layout that needs it is then refused).
    pub viewport: Option<[i32; 2]>,
}

/// The viewer's pointer as the game's input reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pointer {
    /// In the laid-out window's space.
    pub at: [f32; 2],
    /// The path of the button the primary button went down on and is still held: enters Down
    /// Over it, and leaves Down only on release, wherever the pointer is.
    pub pressed: Option<String>,
    /// Seconds since the hovered or pressed element entered its state (`Time - AnimationTime`,
    /// the graphics clock at the change).
    pub since: f32,
}

impl Bindings {
    /// The state bound to `element` of the laid-out template, created empty.
    pub fn bind(&mut self, element: &Element) -> &mut Bound {
        self.bind_at(Site::new(element, None))
    }

    /// The state bound to the object built at `site`, created empty.
    pub(super) fn bind_at(&mut self, site: Site) -> &mut Bound {
        self.bound.entry(site).or_default()
    }

    fn at(&self, site: Site) -> Option<&Bound> {
        self.bound.get(&site)
    }
}

/// A rectangle `x, y, width, height`.
pub type Rect = [f32; 4];

/// A clip in whole interface pixels: `x0, y0, x1, y1`.
pub type Clip = [i32; 4];

/// A glyph a text draws: its codepoint, its quad's top-left, its straight RGBA (the colour its
/// vertices carry: the packed text colour), and its page, box in font pixels and texture
/// coordinates ([`native_text::Font::quad`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnGlyph {
    pub ch: char,
    pub at: [f32; 2],
    pub color: [f32; 4],
    pub texture: usize,
    pub size: [i32; 2],
    pub uv: [f32; 4],
}

/// How a part's effect runs: its program, blend state and slot samplers, and the draw's clock
/// inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    pub compiled: Option<super::native_shader::draw::Effect>,
    /// `None`: the technique is the null record, which runs the previous draw's program and
    /// samplers with blending off ([`Technique::Keep`]).
    pub fixture: Option<(FixtureTechnique, Blend, [Option<Sampler>; 6])>,
    /// `Time - AnimationTime`: seconds since the state's change.
    pub since: f32,
    /// `AnimationTime` as a frame blend reads it.
    pub animation: f32,
    pub time: f32,
    /// Whether the draw's constants carry the clock: uploads `Time` and the drawable's
    /// `AnimationTime`; a masked shield's overlay writes zero to both (`b0` bytes 96..104), so
    /// its `since`, `animation` and `time` stay 0 and it never changes on the clock.
    pub clocked: bool,
}

impl Shading {
    /// How long after its drawable's restart this draw keeps changing on the clock alone
    /// ([`Settles`]); a draw that keeps the previous program ([`Settles::Previous`]) takes the
    /// answer of the program the draw before it bound.
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

/// One drawn part of a laid-out window.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// The drawn element's path.
    pub path: String,
    /// The index, among the laid-out window's children, of the child the part is drawn for
    /// (itself or the child window holding it); `None` for the window's own backgrounds.
    pub child: Option<usize>,
    pub kind: PartKind,
    /// The clip of the windows it is drawn in, if any clips.
    pub clip: Option<Clip>,
    /// The program's `Color` (straight RGBA): the element's colour, its alpha times the opacity.
    pub color: [f32; 4],
    pub shading: Shading,
    /// Original geometry and accumulated parent transform for authored vertex programs.
    pub native: Option<Box<super::native_shader::draw::Part>>,
}

/// A floating frame rotated in radians about `pivot`. The container scale transforms both
/// rectangle and pivot about the content origin. Sprite and progress-bar turns follow the same
/// screen direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub rect: Rect,
    pub rotation: f32,
    pub pivot: [f32; 2],
    /// Native local rectangle, measured upward from the sprite's bottom-left.
    pub local: Rect,
}

impl Geometry {
    /// Where the turn lands a point of the unturned rectangle: with `y` down, a counter-clockwise
    /// turn takes `(dx, dy)` from the pivot to `(dx cos + dy sin, dy cos - dx sin)`.
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

    /// The rectangle's top-left, top-right, bottom-right and bottom-left where they land.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let [x, y, w, h] = self.rect;
        [[x, y], [x + w, y], [x + w, y + h], [x, y + h]].map(|point| self.turn(point))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PartKind {
    /// An animated sprite with initialized geometry, frame offsets and active animation
    /// records.
    Sprite {
        sprite: Vec<u8>,
        geometry: Geometry,
        uv: [f32; 4],
        offsets: [f32; 4],
        /// Its masking texture binds slot 5.
        masked: bool,
    },
    /// A cornered or plain tile's pieces: each quad's rectangle and texture coordinates,
    /// unturned.
    Tiles {
        sprite: Vec<u8>,
        quads: Vec<(Rect, [f32; 4])>,
        native_origin: [f32; 2],
        cornered: bool,
    },
    /// A masked shield's flag pass with flag page in slot 0, mask in slot 1, flag atlas
    /// coordinates and the destination rectangle.
    MaskedFlag {
        sprite: Vec<u8>,
        flag: usize,
        flag_coords: [f32; 4],
        rect: Rect,
        native_origin: [f32; 2],
    },
    /// A bound [`Texture`].
    Cell {
        texture: usize,
        masking: Option<Vec<u8>>,
        uv: [f32; 4],
        geometry: Geometry,
    },
    /// A progress bar: its quad, the texture coordinates at its top-left, top-right,
    /// bottom-right and bottom-left (`horizontal` decides which axis `u` runs along, up the bar
    /// when not), `CurrentState` (`(value / steps) * u scale`, in `f32`), its two colours times
    /// the element's (`vFirstColor`, `vSecondColor`), and whether it draws its textures.
    Progress {
        sprite: Vec<u8>,
        geometry: Geometry,
        uv: [[f32; 2]; 4],
        state: f32,
        colors: [[f32; 4]; 2],
        textured: bool,
    },
    /// An inline icon or flag drawn in white with the text technique and modulation colour,
    /// using raw sprite or atlas UVs. It inherits the text projection and uploads no constants
    /// of its own.
    Inline {
        source: InlineSource,
        rect: Rect,
        uv: [f32; 4],
        modulate: [f32; 4],
        projection: [f32; 2],
        /// [`PartKind::Text::font_viewport`].
        font_viewport: Option<[i32; 2]>,
    },
    /// A circular progress bar's fan: about `centre` with radii `radius`, from `rotation`
    /// radians counter-clockwise on screen, one triangle per unit of 100 (`a_k = k * 2 pi / 100
    /// + rotation`, `k` running on across the sectors, each triangle's vertices the previous
    /// rim point, the centre and the next), the sectors' `weights` (a weight below 1 skipped,
    /// its index still taken); vertex coordinates `(cos a, sin a)`, the centre's `(0, 0)`.
    Fan {
        sprite: Vec<u8>,
        centre: [f32; 2],
        radius: [f32; 2],
        rotation: f32,
        weights: [i32; 2],
    },
    /// A sprite drawn with its animation records ([`animation`]), which only a shader pipeline
    /// renders through the compiled path.
    Animated(Box<Animated>),
    /// A text sprite ([`text_sprites`]): its bake, then its frame of the target drawn as a
    /// sprite quad (`uv` its first frame, `offsets` the frame's `Offset`) with the type's state
    /// technique; its `Color` is white with the opacity as alpha. Only a shader pipeline renders
    /// it through the compiled path.
    TextSprite {
        sprite: Vec<u8>,
        bake: Box<text_sprites::TextBake>,
        geometry: Geometry,
        uv: [f32; 4],
        offsets: [f32; 4],
    },
    /// A text object's glyphs with their loaded pages and UVs, scaled about their placed
    /// top-left positions. Modulation carries the text colour and window opacity. `projection`
    /// scales final GUI positions by the GUI-to-font viewport ratio about the viewport origin;
    /// it is one when the font preserves the viewport copied at construction. Compiled draws
    /// project through `font_viewport` directly.
    Text {
        font: Vec<u8>,
        glyphs: Vec<DrawnGlyph>,
        scale: f32,
        modulate: [f32; 4],
        border_color: u32,
        projection: [f32; 2],
        /// The font's own viewport when a component wrote either axis (the other copied
        /// from the graphics state's GUI viewport at construction); `None`
        /// when the font keeps the copy, so the draw projects through the GUI viewport.
        font_viewport: Option<[i32; 2]>,
    },
}

/// What an inline quad of a text samples.
#[derive(Clone, Debug, PartialEq)]
pub enum InlineSource {
    /// A sprite's texture ([`Sheets::sheet`]).
    Sprite(Vec<u8>),
    /// A loaded page (a flag atlas's).
    Texture(usize),
}

/// A laid-out window: its evaluated size, its parts in draw order, where the items the code adds
/// to its grids and overlapping lists go, and what the game draws nothing for.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: [i32; 2],
    pub parts: Vec<Part>,
    pub holders: Vec<Holder>,
    pub regions: Vec<Region>,
    pub diagnostics: Vec<Diagnostic>,
}

/// An evaluated element's rectangle and logical clip, including text boxes with no glyphs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub path: String,
    /// The outermost object which built this part from a separate template.
    pub owner: Option<String>,
    /// A scrollbar action as `(vertical, direction)`.
    pub scroll: Option<(bool, i32)>,
    pub rect: Rect,
    pub clip: Option<Clip>,
    /// Native pointer geometry, kept separately from cropped/scaled draw bounds.
    pub hit: Option<HitGeometry>,
    pub disabled: bool,
}

impl Layout {
    /// Parts and code-added items in their native attach order.
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

/// A grid or overlapping list the layout drew around: the content its items are placed in by
/// [`grid_slots`] or [`list_places`], and how they are drawn there.
#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub path: String,
    /// As [`Part::child`].
    pub child: Option<usize>,
    /// The parent content's size, which the grid's and list's own geometry is evaluated in.
    pub content: [i32; 2],
    /// The content's top-left in the laid-out window: an item's place `p` is drawn at
    /// `origin + scale * p`.
    pub origin: [f32; 2],
    /// The product of the content scales above it.
    pub scale: f32,
    pub hit_scales: Option<Vec<(f32, [i32; 2])>>,
    /// The content's top-left in the laid-out window's logical whole pixels, where no content
    /// scale reaches: what an item's clip moves by.
    pub logical: [i32; 2],
    pub clip: Option<Clip>,
    /// Opacity inherited by holder items: grids synchronize their items with the window
    /// opacity; overlapping lists do not.
    pub opacity: f32,
    /// The index of the first part drawn after the holder's own: its items draw between the
    /// parts before and this one.
    pub before: usize,
    /// The position the code set on the grid or list ([`Bound::position`]).
    pub position: Option<[i32; 2]>,
}

/// The progress drawable's normalized setter clamps the supplied ratio first,
/// scales in double precision, narrows to float and adds a signed half before
/// truncating. Integer setters have a separate upper clamp.
pub fn normalized_progress_value(fraction: f32, steps: i32) -> i32 {
    let value = (f64::from(fraction.clamp(0.0, 1.0)) * f64::from(steps)) as f32;
    truncate_f32(if value < 0.0 {
        value - 0.5
    } else {
        value + 0.5
    })
}

/// Rounds halves away from zero.
fn round(value: f32) -> i32 {
    if value < 0.0 {
        (value - 0.5).ceil() as i32
    } else {
        (value + 0.5).floor() as i32
    }
}

/// Converts percentage coordinates against the parent extent, then rounds.
fn percent(coordinate: Coordinate, parent: i32) -> i32 {
    if coordinate.percent {
        round(coordinate.value.wrapping_mul(parent) as f32 * 0.01)
    } else {
        coordinate.value
    }
}

/// Why a percentage cannot be evaluated: the parent's size is not given.
fn no_parent(what: impl std::fmt::Display) -> String {
    format!(
        "{what} is a percentage or measures from the parent's far edge, and the parent's size \
         is not given: a root with no parent object is sized in the GUI viewport, the device \
         size over the interface scale, which the caller passes"
    )
}

/// Evaluates each position coordinate against the parent, then applies `min` and `max` bounds
/// whose auto bit is clear. It ignores `step` and the coordinate auto bit.
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

/// Evaluates size `block` for an element at `position` in its parent extent. Percentages use
/// the parent; negative lengths measure from the far edge past position and %% removes
/// position. Assigned min/max bounds apply before preserving aspect ratio refits one axis to
/// the unbounded ratio.
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
            // Too narrow: refit the height unless `min` holds it, else widen up to `max`.
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

/// The orientation's offset from the parent's size plus the origo's from the window's own,
/// halves in C integer division.
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

/// The control orientation offset against the parent extent. Half extents use C integer
/// division.
pub fn control_anchor(orientation: Orientation, parent: [i32; 2]) -> [i32; 2] {
    let [w, h] = parent;
    match orientation {
        Orientation::UpperRight => [w, 0],
        Orientation::LowerLeft => [0, h],
        Orientation::LowerRight => [w, h],
        Orientation::Center => [w / 2, h / 2],
        Orientation::CenterUp => [w / 2, 0],
        Orientation::CenterDown => [w / 2, h],
        // Orientation parsing stores neither `center_left` nor `center_right`.
        Orientation::UpperLeft | Orientation::CenterLeft | Orientation::CenterRight => [0, 0],
    }
}

/// A container's resolved frame inside its parent: its position (top-left) and size, its
/// content's origin inside it and its content's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub at: [i32; 2],
    pub size: [i32; 2],
    pub content_at: [i32; 2],
    pub content_size: [i32; 2],
}

/// A margin's sides, a percentage of the window's own size.
fn margin_sides(margin: &native::Margin, size: [i32; 2]) -> [i32; 4] {
    [
        percent(margin.left, size[0]),
        percent(margin.top, size[1]),
        percent(margin.right, size[0]),
        percent(margin.bottom, size[1]),
    ]
}

/// Evaluates a window frame in its parent extent, or a parent-independent root placed by the
/// caller. Size uses show_position; position uses evaluated coordinates and compute_anchor,
/// with root_origin for a root. Content begins inside the top-left margins and removes every
/// margin from size.
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
        // A root is laid out from its own top-left; `root_origin` places it.
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

/// How the code places a root window.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootPlacement {
    /// Evaluates the authored position in the parent and applies orientation and origo with
    /// `compute_anchor`.
    Constructed,
    /// Uses the supplied top-left point without orientation or origo.
    Set([i32; 2]),
    /// Adds `compute_anchor` to the supplied point; parent resizing reanchors windows with an
    /// origo.
    Anchored([i32; 2]),
}

/// Where a root window's top-left lands in a parent of `parent` pixels (the GUI viewport when
/// the code gives it no parent object: `construct` takes `graphics state`'s size, the
/// device size over the interface scale). `construct` adds half the root size and takes it off
/// again: the centred coordinate space moves nothing.
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

/// A cornered tile's pieces at size `[w, h]` with border `[bx, by]` over a `texture` of
/// `frames` frames, frame `f` (the drawable's frame, 1 outside `1..=frames`): the four corners,
/// the edges repeated by whole texture spans with a remainder (whose coordinates take no frame
/// offset), and the centre stretched or tiled. Every quad is `(x0, y0, w, h)` from the tile's
/// top-left and its `[u0, v0, u1, v1]`.
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
    // Compare the frame and frame count as unsigned values.
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

/// Resolves the drawable sprite or [`FALLBACK_SPRITE`], returning `None` when neither exists.
/// Layout and lifecycle clocks share this lookup.
pub(super) fn drawn_sprite<T>(
    name: &[u8],
    mut lookup: impl FnMut(&[u8]) -> Option<T>,
) -> Option<(Vec<u8>, T)> {
    [name, FALLBACK_SPRITE]
        .into_iter()
        .find_map(|name| Some((name.to_vec(), lookup(name)?)))
}

/// Whether a button draws only its text object (case c): no sprite, a font, and a text (`text`,
/// the one it shows) other than the empty one, an undecodable text included; with an empty text
/// an empty sprite name draws the fallback.
pub(super) fn text_only(button: &native::Button, sprite: &[u8], text: Option<&str>) -> bool {
    sprite.is_empty() && !button.font.is_empty() && text != Some("")
}

/// The drawable name an element constructs: buttons resolve `spriteType` with [`drawn_sprite`]
/// unless text-only ([`text_only`]); check boxes with a font use text-sprite lookup without
/// fallback ([`Walk::check_box`]); lists resolve their background with fallback. Non-instant
/// edit boxes create a text sprite under the empty name. Other elements return `None`.
pub(super) fn own_sprite(element: &Element) -> Option<(&[u8], bool)> {
    match &element.fields {
        Fields::Icon(icon) => Some((&icon.sprite, true)),
        Fields::Button(button) if element.kind == Kind::Checkbox => {
            Some((&button.sprite, button.font.is_empty()))
        }
        Fields::Button(button) => Some((&button.sprite, true)),
        Fields::ListBox(list) => {
            let name = installed_content_core::parser::c_string(&list.background);
            (!name.is_empty()).then_some((name, true))
        }
        Fields::EditBox(edit) if edit.flags() & 0x2 == 0 => Some((b"", false)),
        _ => None,
    }
}

/// Whether `element` is shown: its visible bit as its instance keeps it (every Show checks the
/// Hide bit first, [`lifecycle::Shown::show`]); in a layout with no lifecycle, as a window its
/// code shows leaves it, every element whose authored `hide` is unset.
fn visible(element: &Element, bound: Option<&Bound>) -> bool {
    bound
        .and_then(|bound| bound.shown)
        .map_or(!element.hide, |shown| shown.visible)
}

/// What a window's own keys need to draw as the game draws it; `Ok(false)` when it draws nothing
/// at all (a fade that never runs).
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
        // Construction starts it transparent, and a nonpositive fade time returns without
        // changing it.
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
    // Any show or hide kind moves the window on the clock: a map icon's is hidden once made
    // (moving to its `hide_position`) and, not `moveable`, snaps to its old target on each show
    // even where its `show_position` is its `position`.
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

/// The clip two clips leave (the larger top-left, the smaller bottom-right).
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

/// The walk of one window's children.
struct Walk<'a> {
    resources: &'a dyn Resources,
    bindings: &'a Bindings,
    parts: Vec<Part>,
    holders: Vec<Holder>,
    regions: Vec<Region>,
    diagnostics: Vec<Diagnostic>,
    refusals: Refusals,
    /// The laid-out window's child being walked ([`Part::child`]).
    top: Option<usize>,
    /// The retained opacity of the window being walked.
    opacity: f32,
    /// What the pointer can hit, in draw order (the first pass).
    hits: Vec<Hittable>,
    /// The topmost button and every check box under the pointer.
    hovered: Option<String>,
    boxes: Vec<String>,
    /// The bitmap fonts' effect, read once.
    text_effect: Option<Result<Effect, String>>,
    /// The size supplied to a background drawable: cornered tiles accept it, plain sprites
    /// ignore it.
    sized: Option<[i32; 2]>,
    /// The step that built the objects being drawn from outside the laid-out template
    /// ([`Site`]), which their runtime state is bound under.
    via: Option<Via>,
    region_owner: Option<String>,
    /// The refusals of the sprites the walk looked up ([`SpriteInfo::refusals`]), noted by
    /// every lookup ([`Walk::info`]), measuring ones included.
    refused: RefCell<Refusals>,
}

/// A drawn control the pointer can hit.
struct Hittable {
    path: String,
    part: usize,
    hit: Hit,
    check_box: bool,
}

/// Button state numbers: Up 1, Down 2, Over 3, Disable 4, with seconds since the state began.
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

    /// The effect technique for this state.
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

/// The shading of a technique of `effect`.
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

/// Lays `window` out: its size, and every part the game draws of it and its children, relative
/// to its top-left. `parent` is the size of the object it is built in (the GUI viewport for a
/// root the code gives no parent object). `Ok(None)` when the window draws nothing at all;
/// every refusal of the window and its subtree otherwise. Callers lay a window out through its
/// [`lifecycle::Instance::layout`], which gives the parent and binds its runtime.
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
    let Some(mut drawn) = walk(None, Vec::new())? else {
        return Ok(None);
    };
    // Resolve pointer hits, then walk again using the resulting control states.
    if let Some(pointer) = &bindings.pointer {
        let (hovered, boxes) = pointer_targets(&drawn.parts, &drawn.hits, pointer)?;
        if hovered.is_some() || !boxes.is_empty() || pointer.pressed.is_some() {
            drawn = walk(hovered, boxes)?.expect("drawn in the first pass");
        }
    }
    Ok(Some(drawn.finish(frame.size)))
}

impl Walk<'_> {
    /// Binds the view clock to parts whose constants carry it ([`Shading::clocked`]).
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
        }
    }
}

/// Finds drawables under the pointer within their clip and frame, excluding pass-through
/// parts. Button Over uses the topmost button; check boxes use any hit. Returns paths for that
/// button and every hovered check box.
fn pointer_targets(
    parts: &[Part],
    hits: &[Hittable],
    pointer: &Pointer,
) -> Result<(Option<String>, Vec<String>), Refusals> {
    let mut hovered = None;
    let mut boxes = Vec::new();
    for hit in hits {
        let part = &parts[hit.part];
        let Some(rect) = part_rect(&part.kind) else {
            continue;
        };
        let inside = |rect: Rect| {
            (rect[0]..rect[0] + rect[2]).contains(&pointer.at[0])
                && (rect[1]..rect[1] + rect[3]).contains(&pointer.at[1])
        };
        let clipped = part.clip.is_some_and(|clip| {
            !inside([
                clip[0] as f32,
                clip[1] as f32,
                (clip[2] - clip[0]) as f32,
                (clip[3] - clip[1]) as f32,
            ])
        });
        if !inside(rect) || clipped {
            continue;
        }
        match hit.hit {
            Hit::Through => continue,
            Hit::Alpha => {
                return Err(vec![Refusal::new(
                    &hit.path,
                    RefusalKind::MissingResource,
                    "is under the pointer and its sprite's transparencecheck tests the frame's \
                     alpha, whose texels are not given",
                )]);
            }
            Hit::Frame => {}
        }
        if hit.check_box {
            boxes.push(hit.path.clone());
        } else {
            hovered = Some(hit.path.clone());
        }
    }
    Ok((hovered, boxes))
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
    /// The runtime state of the object the walk builds from `element` where it is ([`Site`]:
    /// The element, and the step that built it from another template, [`Walk::via`]). Every
    /// read of the walk goes through this.
    fn bound(&self, element: &Element) -> Option<&'a Bound> {
        self.bindings.at(Site::new(element, self.via))
    }
}

impl<'a> Walk<'a> {
    /// The drawable's state-effect restart timestamp used as `AnimationTime`, derived from
    /// view time less the bound `effect_since`. The restart occurs at the first draw after a
    /// state change. A drawable never drawn keeps zero.
    fn effect_clock(&self, bound: Option<&Bound>, state: State) -> f32 {
        let since = bound.and_then(|b| b.effect_since).or(match state {
            State::Down(since) | State::Over(since) => Some(since),
            _ => None,
        });
        since.map_or(0.0, |since| self.bindings.time - since)
    }

    /// Runs `draw` with the objects it builds bound under `via` ([`Site`]).
    fn built<T>(&mut self, owner: &Element, role: Role, draw: impl FnOnce(&mut Self) -> T) -> T {
        let outer = self.via.replace((owner as *const Element, role));
        let outer_owner = self.region_owner.clone();
        if outer_owner.is_none() {
            self.region_owner = Some(owner.path.clone());
        }
        let drawn = draw(self);
        self.via = outer;
        self.region_owner = outer_owner;
        drawn
    }

    /// How far the code moves `element` ([`Bound::offset`]).
    fn offset(&self, element: &Element) -> [i32; 2] {
        self.bound(element)
            .and_then(|bound| bound.offset)
            .unwrap_or([0; 2])
    }

    fn shown(&self, element: &Element) -> bool {
        visible(element, self.bound(element))
    }

    /// The text `element` shows: the code's ([`Bound::text`]), else its authored `key`'s
    /// localisation ([`localised`]); `None`, noted, when the game's decode rejects it.
    fn shown_text(&mut self, element: &Element, key: &[u8]) -> Option<String> {
        if let Some(text) = self.bound(element).and_then(|bound| bound.text.clone()) {
            return Some(text);
        }
        self.authored_text(element, key)
    }

    /// The text an authored `key` shows ([`localised`]); `None`, noted, when the game's decode
    /// rejects it.
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

    /// The bitmap fonts' effect.
    fn text_effect(&mut self) -> Result<Effect, String> {
        let resources = self.resources;
        self.text_effect
            .get_or_insert_with(|| resources.text_effect())
            .clone()
    }

    /// Walks a window's children in attach order. Window scale transforms positions, sizes and
    /// glyphs below it; clipping remains at the unscaled window translation. Refuses
    /// unsupported children while continuing to collect all other reasons.
    fn window(
        &mut self,
        window: &Element,
        origin: [i32; 2],
        frame: Frame,
        clip: Option<Clip>,
    ) -> Result<bool, Refusal> {
        let bound = self.bound(window);
        if !self.shown(window) || !window_draws(window, bound)? {
            return Ok(false);
        }
        let container = window.container().expect("checked by window_draws");
        if self.bindings.regions {
            self.regions.push(Region {
                path: window.path.clone(),
                owner: self.region_owner.clone(),
                scroll: None,
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
        // The content object moves by the scroll its bars' construction notified (the clip does
        // not move with it: `propagate_clip` hands on the window's own rectangle).
        let scroll = self.content_scroll(window, container);
        let offset = self.scroll_offset(window, &scroll);
        // An unset clip byte clips a window with backgrounds; clips to the content, which the
        // margins inset.
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
        // The laid-out window's own children name the parts drawn for them and in them.
        let root = self.top.is_none();
        let (first_part, first_holder, first_region) =
            (self.parts.len(), self.holders.len(), self.regions.len());
        let moved = [
            content[0].wrapping_add(offset[0]),
            content[1].wrapping_add(offset[1]),
        ];
        for (index, child) in window.children.iter().enumerate() {
            // A plain scrollbar hidden by its own `hide` still draws the range icons its
            // constructor showed ([`Walk::scrollbar_child`]).
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
            // The outer wrapper translates before the content object applies
            // its scale. Keep both native traversal steps, even at scale one.
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
        // The scrollbars sit on the outer object after the content, which the content scale
        // does not reach, clipped by the parent's clip only.
        if let Some(opacity) = bound.and_then(|bound| bound.opacity) {
            self.opacity = opacity;
        }
        let drawn =
            self.container_scrollbars(window, container, &scroll, origin, frame, outer_clip);
        self.opacity = outer_opacity;
        drawn?;
        Ok(true)
    }

    /// Each `background` is a `buttonDrag` whose drawable is its sprite, or the fallback
    /// when absent, on the window's outer object, at
    /// its raw `position` from the window's top-left (no orientation, centring, scale or
    /// rotation), its `frame` always set; sizes it to the window, which a plain sprite ignores
    /// (resizing a plain sprite has no effect). A `background_margin` with any side set moves each by
    /// its left and top (percentages of the window). Only the parent's clip reaches them.
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
        for background in &container.backgrounds {
            let Some(button) = background.button() else {
                continue;
            };
            let at = [
                origin[0] + button.position[0] + moved[0],
                origin[1] + button.position[1] + moved[1],
            ];
            // Which construction runs: the window less the margin's sides, less the moved
            // position.
            self.sized = Some([
                frame.size[0] - (left + right) - (button.position[0] + moved[0]),
                frame.size[1] - (top + bottom) - (button.position[1] + moved[1]),
            ]);
            let drawn = self.sprite_part(
                background,
                &button.sprite,
                button.frame,
                1.0,
                0.0,
                at,
                Centre::None,
                clip,
            );
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
            // A grid's and a list's items are windows the code adds ([`grid_slots`],
            // [`list_places`]), placed in the content a holder names.
            Fields::Grid(grid) => {
                // The first grid background is a drag button on the grid position object,
                // drawn below items at its raw position. Construction sizes it once to the
                // authored grid; plain sprites ignore that size.
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
            // Window opacity does not reach overlapping-list items.
            Fields::List(_) => self.hold(child, origin, parent, clip, 1.0, None)?,
            // No instance factory: it draws nothing.
            Fields::PositionType(_) => {}
            Fields::ListBox(list) => self.list_box(child, list, origin, parent, clip)?,
            Fields::EditBox(edit) => self.edit_box(child, edit, origin, parent, clip)?,
            Fields::Scrollbar(bar) => self.scrollbar_child(child, bar, origin, clip)?,
            Fields::ExtendedScrollbar(bar) => {
                self.extended_child(child, bar, origin, parent, clip)?;
            }
            Fields::DropDown(drop) => self.drop_down(child, drop, origin, clip)?,
            Fields::Browser(_) if child.kind == Kind::CurveGraph => self.curve_graph(child)?,
            Fields::Browser(_) => self.browser(child)?,
            // Containers build none of these; the reader retains them as templates or
            // fixed-window children.
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

    /// A grid's or list's [`Holder`].
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
        if self.bindings.regions {
            self.regions.push(Region {
                path: element.path.clone(),
                owner: self.region_owner.clone(),
                scroll: None,
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

    /// The sprite `name` for `path`, as [`Resources::sprite`] gives it, its refusals noted
    /// ([`noted`]): every lookup of the walk goes through this.
    pub(super) fn info(&self, path: &str, name: &[u8]) -> Option<SpriteInfo> {
        self.info_ref(path, name).map(Cow::into_owned)
    }

    fn info_ref(&self, path: &str, name: &[u8]) -> Option<Cow<'a, SpriteInfo>> {
        looked_up_info(self.resources, &self.refused, path, name, false)
    }

    /// An element the walk builds from outside the subtree it was given (a registry template by
    /// name, a part its scrollbar's `parent` template holds, an element a caller measures
    /// alone): the game constructs it from the fields its reader stored, so the reader's
    /// refusals of it and its subtree are this walk's ([`reader_refusals`]), as the laid-out
    /// root's are. Every such element reaches the walk through this.
    pub(super) fn resolved<'b>(&self, found: &'b Element) -> &'b Element {
        let mut refused = self.refused.borrow_mut();
        for refusal in reader_refusals(found) {
            if !refused.contains(&refusal) {
                refused.push(refusal);
            }
        }
        found
    }

    /// The sprite drawn under `name`: its own, else the fallback texture; `None` when neither is
    /// defined ("Fatal Error! No [GFX_default_fallback_texture] defined.": no drawable), which
    /// is noted for `path`.
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

    /// The zero-based frame selects: `F - 1` for `F > 0`, not clamped, the first for 0, and for
    /// a negative `F` the state's (`state - 1`) when within the frame grid,
    /// else the first. Past the strip the texture coordinates run on at the frame's offset,
    /// where the effect's sampler addressing reads them.
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

    /// Places an icon or button-centred drawable, drawing it in the Up state.
    #[allow(clippy::too_many_arguments)]
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

    /// [`Self::sprite_part`] in `state`: the part's index, and the sprite's hit test, when one
    /// is drawn.
    #[allow(clippy::too_many_arguments)]
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
        // A text sprite made with no font (an icon's, a
        // background's, a scrollbar part's) is never baked: it draws nothing.
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
        // An eagerly loaded sprite's active animation records; with none active it draws with
        // its plain technique.
        let records = if info.animated && texture.is_none() {
            self.animation_records(element, &sprite, &info, state)?
        } else {
            Vec::new()
        };
        // Missing textures prevent the sprite draw.
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
        // A bound texture takes the frame's place; the sprite keeps its anchor and effect.
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
            // A progress bar's anchor slot is zero.
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
        // `Color`'s alpha is the element's times the opacity.
        color[3] *= self.opacity;
        let frame_animated = matches!(info.kind, SpriteKind::FrameAnimated { .. });
        let (kind, mut shading) = match info.kind {
            SpriteKind::Plain => {
                // The quad is the frame times the scale from the sprite's bottom-left, which
                // puts at `y + int_h` (`int_h` the truncated scaled height); a
                // stretch `k` keeps `[d, h k + d]` of its height, `d = (h - h k) / 2`. The
                // turn's pivot is the unscaled frame's centre.
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
                        // The strip index converted to floating point and multiplied by the
                        // step, for the current and next frames.
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
                                records: records.clone(),
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
                // Progress bars ignore drawable scale and stretch, selecting Texture or Color
                // independently of button state. Integer assignments clamp above to
                // `steps`; normalized assignments already applied their own setter and
                // render their rounded state directly. Authored icon frames and button
                // states use the integer path.
                let own = match frame {
                    FrameSource::Own(frame) => frame,
                    FrameSource::State(frame) if frame > 0 => frame,
                    FrameSource::State(_) => state.number() as i32,
                };
                let value = match bound_progress_float {
                    Some(value) => value,
                    None => bound_progress.or(bound_frame).unwrap_or(own).min(steps),
                };
                // `(value / steps) * u scale` in `f32` (the u scale is 1 unless textures are
                // forced to powers of two): a zero `steps` divides by zero.
                let state_value = (value as f32 / steps as f32) * 1.0;
                let [w, h] = size.map(|value| value as f32);
                // Spans `[0, w] x [-1, h - 1]` from the bar's bottom-left on Direct3D 11, one
                // pixel below its place, and turns it about that origin. Its vertices'
                // coordinates run `u` along the bar when `horizontal`, up it otherwise.
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
                // Tile setters retain rotation and scale, but drawing uses only inherited
                // container scale and translation, so the tile stays unturned at its own size.
                let _ = rotation;
                // Cornered tiles start at their supplied top-left and draw downward. A
                // background can replace the type size. Frames outside the strip use the first
                // frame; alpha comes from retained tile opacity or pulse.
                let size = self.sized.unwrap_or(size);
                let own = match frame {
                    FrameSource::Own(frame) | FrameSource::State(frame) => frame,
                };
                let mut f = bound_frame.unwrap_or(own);
                if f != 0 && (f > info.frames || f < 1) {
                    f = 1;
                }
                // The looping pulse and window opacity each replace the tile alpha when
                // written.
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
                // `size` up from the bottom-left at `y + int_h` (`int_h` the texture's height),
                // the texture repeated by whole-number `size / texture` spans, converted to floats,
                // with no frame.
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
                            records: records.clone(),
                            masked: info.masked,
                        }))
                    },
                    shading(&effect, state.technique(), state.since()),
                )
            }
            SpriteKind::Resizeable => return Ok(None),
            SpriteKind::FrameAnimated { play_on_show, .. } => {
                // A plain sprite whose frame and `AnimationTime` its clock gives one
                // that plays on show animates on its instance's clock, unless its wrap never
                // ended there.
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
                    // Never played: no time elapsed.
                    None => (None, 0.0),
                };
                // Playing updates the frame; otherwise the control or code-selected frame
                // remains.
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
                // Centres the mask inside the overlay with unrounded half differences, placing
                // both one pixel below the supplied point. The flag pass uses the sprite
                // Effect. The white overlay follows with the content-resolved Up technique and
                // zero Time and AnimationTime.
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
            SpriteKind::PieChart { .. } | SpriteKind::LineChart { .. } => return Ok(None),
            SpriteKind::CircularProgress {
                size,
                rotation: turn,
                amount,
            } => {
                // Circular progress uses remaining and completed weights from authored amount
                // or code-written progress. Normalize the first weight with f32 arithmetic and
                // half-up truncation, then give the last sector the nonnegative remainder to
                // 100.
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
                // Puts the drawable's origin at the bottom-left (the integer extent's height below the
                // place), the fan's centre; the icon's scale scales the rim about it, its
                // rotation does not reach it, and no colour or opacity does.
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
                scroll: None,
                rect,
                clip,
                hit,
                disabled: matches!(state, State::Disable),
            });
        }
        self.parts.push(Part {
            path: path.clone(),
            child: self.top,
            kind,
            clip,
            color,
            shading,
            native: None,
        });
        Ok(Some((self.parts.len() - 1, info.hit)))
    }

    /// A missing technique retains the preceding program but uses this drawable's constants.
    /// Its elapsed state-effect clock comes from the first draw after a state change;
    /// [`Bound::effect_since`] supplies it for every state.
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
        // `center_on_anchor` takes precedence over `centerPosition`.
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
        // An icon the code disables draws its drawable's Disable state; enabling it, Up.
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

    /// Lays out a button with its own or inherited sprite, orientation offset, optional
    /// centring, scale, frame and state. Positive explicit frames override state selection.
    /// Font text uses the unscaled, uncentred position, and a text-only button uses just its
    /// text object.
    #[allow(clippy::too_many_arguments)]
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
        // `parent` is already resolved: the reader's post-load pass copies the parent's sprite
        // before any button is constructed.
        let sprite = match made(bound, &button.sprite, self.resources) {
            Some((sprite, _)) => sprite,
            None => button.sprite.as_slice(),
        };
        // Code-written text replaces the authored localisation. Undecodable bytes produce no
        // text draw.
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
        // Enabling or disabling changes the state as a pointer does: the drawable's
        // Effect clock restarts at the next draw (its instance keeps it); the Up and Disable
        // programs of the shipped button-state effects read no time.
        let state = if !enabled {
            State::Disable
        } else if pointer.is_some_and(|pointer| pointer.pressed.as_deref() == Some(path)) {
            State::Down(pointer.map_or(0.0, |pointer| pointer.since))
        } else if self.hovered.as_deref() == Some(path) {
            State::Over(pointer.map_or(0.0, |pointer| pointer.since))
        } else {
            State::Up
        };
        // A text the object cannot decode is still a text: its object draws nothing.
        let text_only = text_only(button, sprite, text.as_deref());
        // The text technique of the state, when the sprite's effect declares it.
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
        // Button sprite replacement prepares text with format 2 when text and font are
        // present, otherwise resolving an ordinary drawable with fallback. It then restores a
        // frame of at least 1, raw position, centring on the unscaled extent, and no rotation.
        // Later frame or rotation calls override those properties.
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
            // Only a positive authored frame overrides state selection; otherwise the drawable
            // uses its state.
            let frame = if button.frame > 0 { button.frame } else { -1 };
            (frame, button.rotation, at)
        };
        let rotation = bound.and_then(|bound| bound.rotation).unwrap_or(turn);
        // A registered `textSpriteType` (looked up with no fallback) and a font: bakes the text
        // into it (format 2, the box its own, the border `borderSize`, `multiline`; a text the
        // code sets bakes again in the button's `format`), and no text object is made.
        // `centerPosition` centres it on its text's measure.
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
                });
            }
            return Ok(());
        }
        let mut frame_size = None;
        if !text_only {
            // An empty sprite with a font draws the fallback.
            let drawn = [sprite, FALLBACK_SPRITE]
                .into_iter()
                .find_map(|name| self.info_ref(path, name));
            // A fallback `textSpriteType` with a font: bakes an empty text into it (format 2,
            // no box, no border), its texture alone; the text object follows as for any sprite.
            // Its measured extent before that bake is 0 by 0.
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
            // Case c: sized by `size`, no border, the button's orientation, which the font
            // draw applies again from its stored orientation.
            common(button.size, [0, 0], button.orientation.unwrap_or_default())
        } else {
            // Case a uses the unscaled drawable extent less the border, with orientation 0.
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

    /// Lays out a check box using sprite, position, font and orientation. Its sprite is
    /// unscaled and uncentred, with Up when unchecked, Down when checked and Over when hovered
    /// unchecked. A window opacity below one hides it.
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
        // A setter's name the registry lacks, or the one shown, changes nothing ([`made`]), as
        // [`Walk::drawable_extent`] measures it.
        let sprite = made(bound, &button.sprite, self.resources)
            .map_or(button.sprite.as_slice(), |(sprite, _)| sprite)
            .to_vec();
        let checked = bound.and_then(|bound| bound.checked).unwrap_or(false);
        let pointer = self.bindings.pointer.as_ref();
        let hovered = self.boxes.iter().any(|hovered| hovered == path);
        let state = if checked && hovered {
            State::Down(pointer.map_or(0.0, |pointer| pointer.since))
        } else if checked {
            // A code-written check sets state 2 and restarts the drawable effect at the next
            // draw, retained by its instance.
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
            State::Over(pointer.map_or(0.0, |pointer| pointer.since))
        } else {
            State::Up
        };
        let anchor = control_anchor(button.orientation.unwrap_or_default(), parent);
        let at = [
            origin[0] + button.position[0] + anchor[0],
            origin[1] + button.position[1] + anchor[1],
        ];
        // With a font, (format 0, no border, fixed): a name the registry lacks registers a new
        // untextured `text sprite type`, whose bake has no width and draws nothing; a
        // registered `textSpriteType` bakes the text; any other sprite drops it.
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
            });
        }
        Ok(())
    }

    /// An instant text box lays out authored or code-written text in its box with border,
    /// format, alignment and orientation. A named scrollbar ([`Walk::named_plain`]) resizes to
    /// box height and refreshes on every text write. Fixed-point overflow determines its range
    /// and hidden flag. Its initial value clamps through the limits and notifies the text
    /// offset. The text clip replaces the window clip with the inner box; the scrollbar draws
    /// afterwards without window opacity.
    fn text_box(
        &mut self,
        element: &Element,
        text_box: &native::TextBox,
        origin: [i32; 2],
        parent: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<(), Refusal> {
        // Moves the text and its scrollbar with the bound position.
        let graphics_origin = origin;
        let offset = self.offset(element);
        let origin = [0, 1].map(|axis| origin[axis].wrapping_add(offset[axis]));
        // Makes 0 a 200; `%u` values past `i32` are the same bits as the game's signed box.
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
                scroll: None,
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
                    // In the parent's content, add scroll to the text's vertical position
                    // before truncating.
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

    /// Prepares text for a fixed or single-line box by clipping to the inner box and appending
    /// three dots when truncated.
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

    /// Measures text prepared from `text`, with no height limit and wrapping at box width.
    /// Undecodable bytes measure nothing. Border, special sequences and word breaks use the
    /// same operands as drawing.
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

    /// Measures prepared text line heights at box width with unlimited height, nonnegative
    /// border and special sequences enabled. Undecodable bytes measure zero.
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
    /// Builds text parts after clipping fixed or single-line boxes and appending truncation
    /// dots. Layout wraps inside the box less two borders, then applies orientation, format
    /// and vertical alignment. Colour comes from the font, its text colour code, or the bound
    /// tint. `!` restores the font's resting colour; other codes search the font table then
    /// the global table and unknown codes preserve colour. Modulation alpha carries window
    /// opacity.
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
        // Orientation determines the first line top and the alignment anchor against the
        // parent extent.
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
        // The glyphs' vertices collect in one buffer, which the draw submits whenever it passes
        // `0xBFF` vertices (six per drawable glyph: after every `GLYPHS_PER_FLUSH` glyphs) and
        // at the end; an inline icon or flag draws at once where it is reached. A later inline
        // quad therefore covers the glyphs of a batch already submitted, and lies under the
        // ones still collecting.
        let text_part = |glyphs: Vec<DrawnGlyph>| Part {
            path: path.clone(),
            child: None,
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
        // The last submission; one of no vertices draws nothing.
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

/// What one refresh of an `instantTextBoxType`'s scrollbar did to it ([`Walk::text_refresh`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refreshed {
    /// The range is the one the bar has: nothing changes.
    Unchanged,
    /// A new range above 1: the flag is cleared and the bar shown when it was visible or
    /// flagged (`flagged`, the flag as it was).
    Above { flagged: bool },
    /// A new range of 1 or less: the flag set and the bar hidden.
    AtMost,
}

impl<'a> Walk<'a> {
    /// The scrollbar an `instantTextBoxType` builds ([`Walk::named_plain`]) as its constructor
    /// leaves it: resized to the box's height once, refreshed for its authored text
    /// ([`Walk::text_refresh`]), its node beside the box, hidden, and set to 0 through its
    /// limits, whose notification moves the text up by the value clamped into `[0, max]` (a
    /// later range change clamps the value without a notification, so the text keeps that
    /// offset). The bar, the box's scrollbar-hidden flag (set initially until a refresh
    /// changes the range) and the text's scroll; `None` without one.
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

    /// Refreshes the scrollbar immediately after every text write. The range uses fixed-point
    /// text overflow with a minimum of one. Changed limits clamp the value and reposition the
    /// slider; overflow clears the hidden flag and shows the bar when admitted, otherwise the
    /// flag is set and the bar hides.
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

    /// An `instantTextBoxType`'s scrollbar as the walk draws it ([`Walk::text_box`]): the bar as
    /// its instance's text writes left it ([`lifecycle::Instance::set_text`]), or outside an
    /// Instance as its construction and one refresh for the text `bound` sets leave it; whether
    /// it draws (its flag clear and a range of at least 1) and the text's scroll; `None`
    /// without one.
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

/// The per-axis GUI-to-font viewport ratio scales final text positions. It is one for the
/// viewport copied at construction. A component-written font viewport requires
/// [`Bindings::viewport`]; zero values retain the floating-point infinities or NaNs produced
/// by division and continue through drawing.
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

/// A glyph as the draw writes it ([`native_text::Font::quad`]); one on a page whose texture did
/// not load binds the null texture and is noted in `diagnostics`.
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

/// Glyph batches hold `0xC00` vertices, six per glyph, and flush after exceeding `0xBFF`.
pub const GLYPHS_PER_FLUSH: usize = 0xC00 / 6;

/// A button's retained text-sprite bake: text, format, authored box and border.
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

/// Queries the current drawable integer extent through the same lookup as drawing
/// ([`Walk::drawable_extent`]). Returns `None` without a drawable and reports all measurement
/// refusals. Drawing preserves fractional frame sizes ([`SpriteInfo::frame_size`]); callers
/// deriving offsets use this integer query.
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
    /// [`drawable_extent`]: the drawable the element shows ([`made`], as [`Walk::state_part`]
    /// draws it): the authored one, or the one a setter made from a name the registry defines
    /// (return before touching the drawable otherwise, so a failed replacement keeps the
    /// current one). Its extent: a sprite's ([`SpriteInfo::scaled_extent`] at the element's
    /// scale, a check box's 1, restored by every sprite setter), a bound texture's in
    /// its frame's place; a text-only button's text object; a text sprite's bake (the target's
    /// width over `noOfFrames`, unsigned, by its height).
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
            // A text-only button's drawable is its text object (case c): its box `size` (0 made
            // 200), no border.
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
        // With a font, a registered text sprite is baked (no fallback is tried for it).
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
            // Registers a new untextured `text sprite type` under the name, whose bake has no
            // width ([`Walk::check_box`] draws nothing of it).
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
        // A fallback text sprite with a font is baked empty: 0 by 0 before and after.
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

    /// Measures the text-sprite bake prepared by a button or check box.
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
        // The bake width divided by `noOfFrames`, unsigned; a type of no frames never bakes (its quad's
        // division is refused first).
        let frames = info.frames as u32;
        Ok([(bake.size[0] as u32 / frames) as i32, bake.size[1]])
    }
}
/// Where a sprite control's frame comes from: an icon's or background's own (as it is), or a
/// button's (above 0, else the drawable's -1 that follows the state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameSource {
    Own(i32),
    State(i32),
}

/// Looks up the font named by a text object. No registered font causes a null read during
/// construction. Fonts refused by content or resource loading report their own reasons
/// ([`native_text::Font::refusals`]).
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

/// `info`, the sprite `name` for `path`, its refusals noted once in `refused`.
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

/// An inline icon's texture size and frame count for text layout: `noOfFrames`, or 1
/// for a progress bar or a cornered tile.
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

/// The box size, border and layout flags used to measure a text object.
#[derive(Clone, Copy, Debug)]
struct TextObject {
    /// The box, 0 already made 200.
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

/// Inputs for a text object.
struct TextSpec<'a> {
    /// The font name bytes used for lookup.
    font: &'a [u8],
    text: &'a str,
    /// Its position in the parent's content.
    position: [i32; 2],
    /// Its box, 0 already made 200.
    size: [i32; 2],
    border: [i32; 2],
    format: TextFormat,
    vertical_alignment: VerticalAlignment,
    orientation: Orientation,
    /// The parent's extent the orientation offsets from.
    extent: [i32; 2],
    fixed_size: bool,
    truncate: bool,
    multiline: bool,
    special: bool,
    color_code: u8,
    /// The button-state TextUp, TextOver, TextDisable or TextDown technique. `None` selects
    /// the bitmap font's default Text technique.
    technique: Option<(Effect, &'static str)>,
}

/// The sprites and fonts a template names, which a caller loads before laying it out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assets {
    /// Every sprite name, and [`FALLBACK_SPRITE`], which an empty or unknown name draws.
    pub sprites: BTreeSet<Vec<u8>>,
    /// Every font name, as [`Resources::font`] is asked for it.
    pub fonts: BTreeSet<Vec<u8>>,
    /// Every texture file the code loads by path ([`Resources::texture_file`]).
    pub textures: BTreeSet<Vec<u8>>,
}

impl Assets {
    /// Inline icons resolve through the global sprite registry, including names formed by
    /// localisation substitutions. Shared by GUI consumers.
    pub fn from_localisation(localisation: &Localisation) -> Self {
        let mut assets = Self::default();
        for (key, _) in localisation.iter() {
            assets.add_localised(localisation, key.as_bytes());
        }
        assets
    }

    /// Adds sprites referenced by displayed text to the load set, trying the GFX_ prefix
    /// before the bare name. Callers do this for every code-bound text ([`Bound::text`]).
    pub fn add_text(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        self.add_chars(&chars);
    }

    /// The inline icons of the text an authored `key` shows, through the same localisation and
    /// decode as the text object draws it ([`localised`]); nothing for bytes the decode rejects,
    /// which draw nothing.
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
                    let (names, _) = native_text::icon_lookup(&name);
                    self.sprites.extend(names);
                    index += length;
                }
                Some((length, _)) => index += length,
                None => index += 1,
            }
        }
    }

    /// Adds every sprite, font and texture `other` names.
    pub fn extend(&mut self, other: Assets) {
        self.sprites.extend(other.sprites);
        self.fonts.extend(other.fonts);
        self.textures.extend(other.textures);
    }
}

/// Every sprite, font and texture file `element` and its kept descendants (backgrounds and
/// scrollbar parts included) draw, found as construction finds them: the templates it names in
/// the global registry (`gui_type`, [`Resources::gui_type`]: a container's scrollbars, a list
/// box's or text box's `scrollbarType`), a plain scrollbar's parts through
/// [`widgets::scrollbar_part`] (its own child, else its `parent` template's) and a
/// `windowType`'s background the same way ([`fixed_window_layout`]); a `textBoxType`'s bake
/// (its own name's sprite, its font and `textureFile`); and the inline icons of every authored
/// text through `localisation` ([`Assets::add_localised`]). A `windowType` root (a list box
/// row's [`ListEntry::Fixed`]) is collected whole this way. A caller adds what the code binds
/// ([`Assets::add_text`], [`Bound::sprite`]) and loads the whole set, every category, before
/// laying the element out.
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
    if seen.contains(&(element as *const Element)) {
        return;
    }
    seen.push(element);
    let mut sprite = |name: &[u8]| {
        if !name.is_empty() {
            assets.sprites.insert(name.to_vec());
        }
    };
    // Templates found by name in the global registry, and parts found where the scrollbar's
    // constructor finds them.
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
            // A text sprite of the empty name over `textureFile`.
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
        // Resolve a fixed-window background in its name tree, then the parent template
        // children.
        Fields::Window(_) => {
            if let Ok(Some(background)) = fixed::background(element, gui_type) {
                found.push(background);
            }
        }
        // Collect the text box name sprite when registered, plus its font, inline icons and
        // TextureFile.
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

/// The window a list box item is made of ([`list_entry`]).
#[derive(Clone, Copy, Debug)]
pub enum ListEntry<'a> {
    /// A `containerWindowType`: the container factory laid out as any window
    /// ([`layout`], [`assets_in`]).
    Container(&'a Element),
    /// A fixed `windowType` entry.
    Fixed(&'a Element),
}

/// Finds the template for a list box item. Containers use ordinary container construction;
/// every other kind enters the fixed-window builder without a kind check. Only a window
/// template is valid there; other kinds refuse the resulting type-confused read. Absent
/// templates select an undescribed GUI-owned object and are refused. Fixed entries use
/// [`fixed_window_layout`] through their [`lifecycle::Instance`], and asset collection uses
/// [`assets_in`].
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

/// How a sprite control is centred on its position.
#[derive(Clone, Copy)]
enum Centre {
    None,
    /// `centerPosition`: less half the drawable's extent.
    Position,
    /// Centres on half the unscaled drawable extent using C integer division.
    Unscaled,
    /// `center_on_anchor`: less the sprite's `anchor`.
    Anchor,
}

/// A grid's items as places them, and where the grid's own object is (its items are drawn
/// relative to it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridLayout {
    /// The constructed grid top-left in parent content, including its orientation offset and
    /// no finalize adjustment.
    pub origin: [i32; 2],
    /// The relaid-out grid top-left includes finalize adjustments. Finalize alone does not
    /// move it; callers use this only after relaying out the filled grid.
    pub relaid: [i32; 2],
    /// Each appended item window's top-left relative to the grid object: its slot plus the
    /// offset its construction saved, adding each `i32` with wrapping. Automatic append limits
    /// can leave items out; explicit positions bypass those limits.
    pub items: Vec<[i32; 2]>,
    /// The grid's extent after finalize (the authored size plus the finalize adjustments); the
    /// background keeps the authored size it was given at construction.
    pub extent: [i32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridBounds {
    adjust: [i32; 2],
    extent: [i32; 2],
}

/// The item offset saved at construction without a parent: evaluate position in the GUI
/// viewport and apply `compute_anchor`. Appending an item adds this offset to its slot.
#[cfg(any(test, feature = "test-support"))]
pub fn item_offset(window: &Element, viewport: [i32; 2]) -> Result<[i32; 2], String> {
    Ok(container_frame(window, Some(viewport))?.at)
}

/// The next grid slot after `last`, with `(0, 0)` first. Fill order chooses across or down;
/// positive limits reject slots beyond them, while nonpositive limits are unrestricted.
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

/// A slot position relative to a grid of `extent` with cells of `cell`.
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

/// A grid's position, authored extent and cell size.
type GridGeometry = ([i32; 2], [i32; 2], [i32; 2]);

/// The evaluated grid position and authored extent against its parent, plus cell size
/// evaluated against the grid's own extent.
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

/// The places `count` items take in `grid` relative to its parent's content, in a parent of
/// `parent` pixels, before each item's own offset: [`finalize_grid`]'s slots from the grid's
/// constructed place, or from `set`, the position the code set ([`Holder::position`]).
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

/// Finalizes appended item windows in order. Each item evaluates its initial position and
/// anchor in the GUI viewport and saves that offset. Slot placement starts with zero
/// adjustments and authored grid extent, then measures item bounds from the origin. Fixed
/// padding and measured overflow determine retained adjustments; percentage padding
/// contributes zero.
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

/// Finalizes explicitly positioned items, bypassing automatic append limits.
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
        // The saved offset: the item's position, anchor and size against the graphics root,
        // which reparenting to the grid (the
        // wrapper's parent only) does not change.
        let frame = container_frame(item, Some(viewport))
            .map_err(|why| Refusal::new(&item.path, RefusalKind::NoParent, why))?;
        // `item_offset`.
        let offset = frame.at;
        let at = [
            place[0].wrapping_add(offset[0]),
            place[1].wrapping_add(offset[1]),
        ];
        placed.push((at, frame.size));
    }
    finalize_placed(element, grid, parent, size, &placed, set)
}

/// Finalizes item bounds after the caller resolves slots and item offsets.
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

/// Computes the grid top-left by adding evaluated position, finalize adjustment and
/// orientation offset. Half extents use C integer division; unsupported orientations
/// contribute no offset.
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
    /// Each appended item's place in the grid's parent content, ready for [`Holder::place`]:
    /// The grid object's constructed place plus the item's place relative to it (each `i32`
    /// wrapping).
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
    /// [`finalize_grid`] of the grid this holder is, over the item windows the code appends in
    /// order (constructed in the GUI viewport of `viewport` pixels): each placed item's place
    /// in the holder's content ([`GridLayout::places`]), from the holder's content and the
    /// position the code set ([`Holder::position`]).
    #[cfg(any(test, feature = "test-support"))]
    pub fn finalize(
        &self,
        grid: &Element,
        viewport: [i32; 2],
        items: &[&Element],
    ) -> Result<Vec<[i32; 2]>, Refusal> {
        Ok(finalize_grid(grid, self.content, viewport, items, self.position)?.places())
    }

    /// The opacity a code-added item window takes: a grid sets its own on every item,
    /// overwriting theirs; an overlapping list sets none. A caller binds it as the item
    /// window's [`Bound::opacity`] before laying the item out.
    #[cfg(any(test, feature = "test-support"))]
    pub fn item_opacity(&self) -> Option<f32> {
        (self.opacity != 1.0).then_some(self.opacity)
    }

    /// Places an item layout at its holder-relative point. Translation and holder scale move
    /// every part; the holder clip intersects the item clip, whose translation remains
    /// unscaled.
    pub fn place(&self, place: [i32; 2], item: &Layout) -> Vec<Part> {
        self.place_parts(place, &item.parts)
    }

    /// Regions follow the holder's scale and translation; clips move in logical pixels.
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
                    scroll: region.scroll,
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

    /// A list box's scrollbar ([`widgets::ListBoxRows::scrollbar`], in the list box's parent
    /// content) placed as the holder's items are, after its rows; a smooth list box's escapes
    /// the holder's clip.
    pub fn place_scrollbar(&self, rows: &widgets::ListBoxRows) -> Vec<Part> {
        let mut parts = self.place_parts([0, 0], &rows.scrollbar);
        if !rows.clipped() {
            for part in &mut parts {
                part.clip = None;
            }
        }
        parts
    }

    /// `parts` placed at `place` in the holder's content, as [`Holder::place`] places an item's.
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
        // Clips are the game's logical whole pixels, which no content scale reaches.
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

/// Draws window parts and holder items in attach order. Each holder's already placed items
/// ([`Holder::place`]) appear between its preceding and following parts. Windows draw their
/// own parts even without items. Submission is all or nothing, as [`draw`].
pub fn draw_composed(
    layout: &Layout,
    items: &[Vec<Part>],
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
) -> Result<(), Refusal> {
    let _submit_profile = crate::profiling::span("dc_gui_submit");
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

/// The list object's oriented origin.
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

/// Places overlapping-list items in paint order, respecting first_on_top and the list
/// orientation. Each result is `(index, place)`. `parent` is the window content extent, or the
/// GUI viewport for an independently created list.
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
        // Compute the step in f32, then add the double-precision half before truncating the
        // pixel position.
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

/// Integer conversion truncates toward zero, yielding `0x80000000` for NaN or an out-of-range
/// value (a single entry wider than its list divides by zero, and `0 * -inf` is NaN).
fn truncate(value: f64) -> i32 {
    if value.is_nan() || value <= f64::from(i32::MIN) - 1.0 || value >= f64::from(i32::MAX) + 1.0 {
        i32::MIN
    } else {
        value as i32
    }
}

/// The sheets and fonts [`draw`] draws from.
pub trait Sheets {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
    /// A progress bar's `textureFile1` and `textureFile2`.
    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]>;
    /// A sprite's `masking_texture` ([`SpriteInfo::masked`]).
    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
    /// A masked shield's mask (`textureFile2`); its overlay is [`Sheets::sheet`].
    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet>;
}

/// The program, the samplers of slots 0, 1 and 5, and their biases, the last draw of a window
/// ran with: what a null technique or an undeclared sampler keeps bound.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Kept {
    technique: u32,
    samplers: [Option<u32>; 3],
    lod_bias: [f32; 3],
}

/// Pushes layout parts at the root's whole-pixel top-left `origin`, or rolls back the
/// submission and returns the refusal.
pub fn draw(
    layout: &Layout,
    origin: [i32; 2],
    sheets: &dyn Sheets,
    batch: &mut Batch,
) -> Result<(), Refusal> {
    let _submit_profile = crate::profiling::span("dc_gui_submit");
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

/// Pushes one part's quads; `previous` is what the window's last draw left bound, which the
/// part updates.
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
    // The technique's program and samplers, or the previous draw's under the null record.
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
                    // An undeclared sampler preserves the preceding binding for that slot.
                    None => {
                        let kept = previous.and_then(|previous| {
                            Some((previous.samplers[index]?, previous.lod_bias[index]))
                        });
                        match kept {
                            Some((sampler, bias)) => {
                                bound.samplers[index] = Some(sampler);
                                bound.lod_bias[index] = bias;
                            }
                            // Slot 0 is always read; the others only by programs that read them.
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
    // A text's projection ([`PartKind::Text`]) scales its GUI position (the window's origin
    // added) about the viewport's top-left; `exact` adds the origin back.
    let project = |corners: [[f32; 2]; 4], [fx, fy]: [f32; 2]| {
        corners.map(|[x, y]| [(x + ox) * fx - ox, (y + oy) * fy - oy])
    };
    let quad = |texture: usize, color: [f32; 4], exact: Exact| Quad {
        texture,
        rect: [0.0; 4],
        uv: [0.0; 4],
        color,
        rotation: 0.0,
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
