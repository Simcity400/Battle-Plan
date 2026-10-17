//! The game's reading of `interface/**/*.gfx`: all stored sprite fields, fonts and color
//! tables, read token by token with the GUI reader's lexer, key loop, scanners and variables.
//!
//! The file's persistent loop reads `spriteTypes` (0x35), `objectTypes` (0x37) and
//! `bitmapfonts` (0x128) without checking their values; other keys are unexpected and skip a
//! block. Within them, the graphics file reader builds and reads each kind. Sprite declarations
//! are pushed onto their registry bucket's front, so case-sensitive lookup finds the latest.
//! Object kinds (`arrowType`, `animatedmaptext`, `pdxmesh`, `pdxparticle`) keep their token
//! walks and common names in a separate registry. Fonts, overrides and `textcolors` use
//! [`super::fonts`]; global `cursor_offset` and `selection_offset` are also read. Other keys
//! are unexpected. Each sprite reader hands unmatched keys to common `name`, `loadType` and
//! `norefcount` handling; remaining keys report an unexpected token and skip a block.

use std::{collections::BTreeMap, io};

use crate::parser::{native::Kind as TokenKind, read_bool};

use super::{
    Cause, Refusal, TextBlock,
    fonts::{FontEvent, TextColors},
    keywords::{self, Class, DynamicTokens},
    reader::{Axis, FILE, Reader, display, keyword_of, read_string, record},
    spill::{self, CxxString, Layout, Vector, VectorSite},
};
use crate::ResolvedLayers;

/// The default effect for plain sprites, cornered tiles and text sprites, restored when their
/// authored effect is empty during resource initialization.
pub const DEFAULT_EFFECT: &[u8] = b"gfx/FX/buttonstate.lua";

/// Degrees to radians as the readers multiply.
const RADIANS: f32 = 0.017_453_292;

/// One `animation` block: a 0x90-byte record with the sprite reader's defaults, retained when
/// `animationtype` is set.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationData {
    /// `animationmaskfile` and `animationtexturefile`.
    pub mask_file: Vec<u8>,
    pub texture_file: Vec<u8>,
    /// `animationrotation`: `%f` into a zeroed local times the radian factor.
    pub rotation: f32,
    /// `animationlooping` (1).
    pub looping: bool,
    /// `animationtime` (1.0) and `animationdelay` (0).
    pub time: f32,
    pub delay: f32,
    /// `animationtexturescale`: the reciprocal of each component (1.0 for 0).
    pub texture_scale: [f32; 2],
    /// `animationrotationoffset`.
    pub rotation_offset: [f32; 2],
    /// `animationtype`: `scrolling` 1, `rotating`/`rotating_cw` 2, `rotating_ccw` 3, `pulsing` 4,
    /// lowered; a third `animationrotationoffset` component writes its bits.
    pub kind: i32,
    /// `animationblendmode`: `add` 0, `multiply` 1, `overlay` 2, `normal` 3.
    pub blend_mode: i32,
    /// `animationframes` (all): each listed state's bit (`1 << (v & 31)`).
    pub frames: u32,
    /// `animationtriggerframes`: ORed across repeats.
    pub trigger_frames: u32,
    /// The texture's texel size (0, 0), used to scale record UVs. Animation initialization writes
    /// the loaded texture's size when `animationtexturefile` is set; otherwise it retains the
    /// eleventh and twelfth `animationrotationoffset` components.
    pub texture_size: [i32; 2],
}

impl Default for AnimationData {
    fn default() -> Self {
        Self {
            mask_file: Vec::new(),
            texture_file: Vec::new(),
            rotation: 0.0,
            looping: true,
            time: 1.0,
            delay: 0.0,
            texture_scale: [1.0; 2],
            rotation_offset: [0.0; 2],
            kind: 0,
            blend_mode: 0,
            frames: u32::MAX,
            trigger_frames: 0,
            texture_size: [0; 2],
        }
    }
}

/// A plain sprite's fields and defaults: `spriteType` or `quadTextureSprite`, and the base of
/// frame-animated, tile and masked-shield kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct Plain {
    /// `textureFile`.
    pub texture: Vec<u8>,
    /// `noOfFrames` (`%i`, 1), unchecked.
    pub frames: i32,
    /// Every kept `animation` block, in order.
    pub animations: Vec<AnimationData>,
    /// `defaultAnimationTime` (`%f`, 1.0).
    pub default_animation_time: f32,
    /// `effectFile`, as read; [`Plain::effect`] is what the game loads.
    pub effect_file: Vec<u8>,
    /// `transparencecheck`, and the flag every assignment sets.
    pub transparence_check: bool,
    pub transparence_check_set: bool,
    /// `clicksound`.
    pub click_sound: Vec<u8>,
    /// `allwaystransparent`/`alwaystransparent`: frame hit test passes the pointer through.
    pub always_transparent: bool,
    /// `can_be_lowres`.
    pub can_be_lowres: bool,
    /// The frame direction (0): `framedirectiony` stores a boolean read into a zeroed local as 0 or
    /// 1; values other than `yes` or `no` store 0. A tenth `anchor` component writes its bits.
    /// Frame dimensions and geometry run down the texture only for 1. Geometry writes the frame
    /// step for 0, 1 and 2 (both axes), retaining [`Plain::frame_step`] for other values.
    pub frame_direction: i32,
    /// U and V frame steps (0, 0), used to scale the frame offset. Geometry rewrites them for
    /// direction 0, 1 or 2; otherwise the seventh and eighth `anchor` components remain.
    pub frame_step: [f32; 2],
    /// The U flip (0), written by the low byte of an eleventh `anchor` component: geometry mirrors
    /// U when nonzero. Plain sprite draws replace it with the drawable's flip before each draw;
    /// frame animation draws do not.
    pub flip_u: bool,
    /// The texture's registered id (-1), which a 47th `anchor` component writes: The base texture
    /// loading reads a lazily loaded sprite's texture only while it is -1.
    pub texture_id: i32,
    /// `masking_texture`.
    pub masking_texture: Vec<u8>,
    /// `legacy_lazy_load`: the startup flag, set to 1 during application startup.
    pub legacy_lazy_load: bool,
    /// `dx_offset` ((-0.5, 0.5)): added to each vertex on graphics backend 0.
    pub dx_offset: [f32; 2],
    /// `anchor`, which icon position update subtracts under `center_on_anchor`. Further components
    /// write on with no bound (`super::spill`).
    pub anchor: [f32; 2],
    /// The vertical stretch (1.0), written by a fourth `anchor` component. Plain sprite draws
    /// replace it and [`Plain::v_scale`] with the drawable's values before every draw; frame
    /// animation draws do so only when the drawable's values are not both 1.0.
    pub stretch: f32,
    /// The frame step's V scale (1.0), which a fifth `anchor` component writes.
    pub v_scale: f32,
    /// `generate_mip_maps`.
    pub generate_mip_maps: bool,
}

impl Default for Plain {
    fn default() -> Self {
        Self {
            texture: Vec::new(),
            frames: 1,
            animations: Vec::new(),
            default_animation_time: 1.0,
            effect_file: DEFAULT_EFFECT.to_vec(),
            transparence_check: false,
            transparence_check_set: false,
            click_sound: Vec::new(),
            always_transparent: false,
            can_be_lowres: false,
            frame_direction: 0,
            frame_step: [0.0; 2],
            flip_u: false,
            texture_id: -1,
            masking_texture: Vec::new(),
            legacy_lazy_load: true,
            dx_offset: [-0.5, 0.5],
            anchor: [0.0; 2],
            stretch: 1.0,
            v_scale: 1.0,
            generate_mip_maps: false,
        }
    }
}

impl Plain {
    /// The effect the game loads: `effectFile`, or the default when it is empty
    /// (animation initialization).
    pub fn effect(&self) -> &[u8] {
        if self.effect_file.is_empty() {
            DEFAULT_EFFECT
        } else {
            &self.effect_file
        }
    }

    /// Whether the frames run down the texture (frame dimensions calculation compares the frame
    /// direction with 1).
    pub fn frame_direction_y(&self) -> bool {
        self.frame_direction == 1
    }

    /// `(columns, rows)` of frame dimensions calculation: the frame count across, or down when the
    /// direction is 1 (the row count is 1).
    pub fn grid(&self) -> [i32; 2] {
        if self.frame_direction_y() {
            [1, self.frames]
        } else {
            [self.frames, 1]
        }
    }

    /// The frame step used to draw, given the step geometry computes along the strip (`along`,
    /// U for columns and V for rows): `(along, 0)` for direction 0, `(0, along)` for 1, both
    /// for 2. Other directions retain [`Plain::frame_step`] without rewriting it.
    pub fn draw_step(&self, along: [f32; 2]) -> [f32; 2] {
        match self.frame_direction {
            0 => [along[0], 0.0],
            1 => [0.0, along[1]],
            2 => along,
            _ => self.frame_step,
        }
    }
}

/// A `frameAnimatedSpriteType`'s own fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameAnimation {
    /// Seconds per frame (1.0): `animation_rate_spf` as read, `animation_rate_fps` inverted after
    /// its scan (a failed scan inverts the old value).
    pub seconds_per_frame: f32,
    /// `looping` and `play_on_show`.
    pub looping: bool,
    pub play_on_show: bool,
    /// `pause_on_loop` (`%f`).
    pub pause_on_loop: f32,
}

impl Default for FrameAnimation {
    fn default() -> Self {
        Self {
            seconds_per_frame: 1.0,
            looping: false,
            play_on_show: false,
            pause_on_loop: 0.0,
        }
    }
}

/// A `maskedShieldType`'s fields, checked before plain sprite fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaskedShield {
    /// `textureFile1` and `textureFile2`.
    pub texture1: Vec<u8>,
    pub texture2: Vec<u8>,
    /// `effectFile`; the plain sprite's own effect file keeps its default.
    pub effect_file: Vec<u8>,
    /// `small`: flag atlas selection takes the small flag atlas.
    pub small: bool,
}

/// A `corneredTileSpriteType`'s fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct CorneredTile {
    /// `noOfFrames` (`%u`, 1).
    pub frames: u32,
    pub texture: Vec<u8>,
    /// `size` and `borderSize`: integer point reader into the field.
    pub size: [i32; 2],
    pub border: [i32; 2],
    /// `effectFile`; [`CorneredTile::effect`] is the resolved effect loaded for it.
    pub effect_file: Vec<u8>,
    pub always_transparent: bool,
    /// `tilingCenter`.
    pub tiling_center: bool,
    /// The animation rate: `animation_rate_fps` as read, `animation_rate_spf` inverted; `None`
    /// while unwritten (the constructor leaves it uninitialised).
    pub animation_rate: Option<f32>,
    /// `looping`.
    pub looping: bool,
}

impl CorneredTile {
    pub fn effect(&self) -> &[u8] {
        if self.effect_file.is_empty() {
            DEFAULT_EFFECT
        } else {
            &self.effect_file
        }
    }
}

/// A `progressbartype`'s fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// `textureFile1` and `textureFile2`: resource construction uses the "Texture" technique when
    /// both are set, else "Color".
    pub texture1: Vec<u8>,
    pub texture2: Vec<u8>,
    /// `size`: integer point reader into the field; size lookup in every mode.
    pub size: [i32; 2],
    /// `color` and `colorTwo`: color reader, white.
    pub color: [f32; 4],
    pub color_two: [f32; 4],
    /// `effectFile`, as read.
    pub effect_file: Vec<u8>,
    /// `horizontal` (1).
    pub horizontal: bool,
    pub always_transparent: bool,
    /// `steps` (`%i`, 100).
    pub steps: i32,
}

/// A `textSpriteType`'s fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSprite {
    /// `noOfFrames` (`%u`, 1).
    pub frames: u32,
    pub texture: Vec<u8>,
    /// `defaultAnimationTime` (`%f`, 1.0).
    pub default_animation_time: f32,
    /// `effectFile`; empty text loads the default.
    pub effect_file: Vec<u8>,
    pub click_sound: Vec<u8>,
}

/// A `resizeableSpriteType`'s fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Resizeable {
    /// The frame count (`%u`, 1): `noOfFrames` and `defaultAnimationTime` both write it.
    pub frames: u32,
    pub texture: Vec<u8>,
    /// `size`: integer point reader into the field; the drawable's size.
    pub size: [i32; 2],
}

/// A `circularProgressBarType`'s fields and defaults; its shader is
/// `gfx/FX/circularprogressbar.shader`, technique `CircularProgressBar`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CircularProgress {
    pub texture1: Vec<u8>,
    pub texture2: Vec<u8>,
    /// `size` (`%i`): a scalar.
    pub size: i32,
    /// `rotation`: `%f`, then the field times the radian factor, even when the scan stored nothing.
    pub rotation: f32,
    /// `amount` (`%f`).
    pub amount: f32,
}

/// A `PieChartType`'s fields and defaults; its effect is `gfx/FX/color.lua`, technique `Color`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PieChart {
    /// `size` (`%i`), which the constructor leaves uninitialised: `None` until written.
    pub size: Option<i32>,
    /// `colors`: one colour per entry, `rgb` a color object, `hsv` a color object then The
    /// HSV-to-RGB conversion, anything else the current colour again.
    pub colors: Vec<[f32; 4]>,
}

/// A `LineChartType`'s fields and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct LineChart {
    /// `size`: integer point reader into the field.
    pub size: [i32; 2],
    /// `effectFile` (`gfx/FX/linechart.lua`).
    pub effect_file: Vec<u8>,
    /// Half the line width: `linewidth` read by fixed5 reader into a zeroed `i64`, then `(float)v /
    /// 100000 * 0.5` (0.5).
    pub half_width: f32,
}

/// A sprite kind, by the reader that reads it, with every field it stores.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteKind {
    /// `spriteType` (0x36) or `quadTextureSprite` (0x84).
    Plain(Plain),
    /// `frameAnimatedSpriteType` (0x1C8).
    FrameAnimated {
        plain: Plain,
        animation: FrameAnimation,
    },
    /// `tileSpriteType` (0x150): the plain keys and `size`.
    Tile { plain: Plain, size: [i32; 2] },
    /// `maskedShieldType` (0x10F), built by graphics resource reader.
    MaskedShield { plain: Plain, shield: MaskedShield },
    /// `corneredTileSpriteType` (0x55).
    CorneredTile(CorneredTile),
    /// `progressbartype` (0x112).
    Progress(Progress),
    /// `textSpriteType` (0x47).
    Text(TextSprite),
    /// `resizeableSpriteType` (0x46).
    Resizeable(Resizeable),
    /// `circularProgressBarType` (0x310).
    CircularProgress(CircularProgress),
    /// `PieChartType` (0x153).
    PieChart(PieChart),
    /// `LineChartType` (0x168).
    LineChart(LineChart),
}

impl SpriteKind {
    /// The kind's keyword, as the lexer lowers it.
    pub fn keyword(&self) -> &'static str {
        match self {
            Self::Plain(_) => "spritetype",
            Self::FrameAnimated { .. } => "frameanimatedspritetype",
            Self::Tile { .. } => "tilespritetype",
            Self::MaskedShield { .. } => "maskedshieldtype",
            Self::CorneredTile(_) => "corneredtilespritetype",
            Self::Progress(_) => "progressbartype",
            Self::Text(_) => "textspritetype",
            Self::Resizeable(_) => "resizeablespritetype",
            Self::CircularProgress(_) => "circularprogressbartype",
            Self::PieChart(_) => "piecharttype",
            Self::LineChart(_) => "linecharttype",
        }
    }

    /// The plain sprite fields of the plain family.
    pub fn plain(&self) -> Option<&Plain> {
        match self {
            Self::Plain(plain)
            | Self::FrameAnimated { plain, .. }
            | Self::Tile { plain, .. }
            | Self::MaskedShield { plain, .. } => Some(plain),
            _ => None,
        }
    }

    fn plain_mut(&mut self) -> Option<&mut Plain> {
        match self {
            Self::Plain(plain)
            | Self::FrameAnimated { plain, .. }
            | Self::Tile { plain, .. }
            | Self::MaskedShield { plain, .. } => Some(plain),
            _ => None,
        }
    }
}

/// Authored arrow fields and constructor defaults. Asset names retain their string-reader
/// bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowDefinition {
    /// `size` (1) and `endAt` (1).
    pub size: f32,
    pub end_at: f32,
    /// Both color object objects start white.
    pub color: [f32; 4],
    pub color_two: [f32; 4],
    /// `height` and `type` are uninitialised until a successful read. Consumers that need either
    /// unset field must refuse it; the movement draw uses neither.
    pub height: Option<f32>,
    pub arrow_type: Option<i32>,
    /// `heading` (0.2).
    pub heading: f32,
    /// `texture`, `normal`, `specular`, `overlay`, `effect`, each initially empty.
    pub texture: Vec<u8>,
    pub normal: Vec<u8>,
    pub specular: Vec<u8>,
    pub overlay: Vec<u8>,
    pub effect: Vec<u8>,
}

impl Default for ArrowDefinition {
    fn default() -> Self {
        Self {
            size: 1.0,
            end_at: 1.0,
            color: [1.0; 4],
            color_two: [1.0; 4],
            height: None,
            arrow_type: None,
            heading: 0.2,
            texture: Vec::new(),
            normal: Vec::new(),
            specular: Vec::new(),
            overlay: Vec::new(),
            effect: Vec::new(),
        }
    }
}

/// An animated map text type: its constructor defaults and the fields its
/// reader sets.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedMapTextDefinition {
    /// `speed`; the constructor's default is the f32 word `0x3C23D70A`.
    pub speed: f32,
    /// `textblock`, updated in place by each block.
    pub text_block: TextBlock,
    /// `position`, initially zero. A fourth component writes `maxwidth`'s bits.
    pub position: [f32; 3],
    /// `maxwidth`, initially -1.
    pub max_width: i32,
}

impl Default for AnimatedMapTextDefinition {
    fn default() -> Self {
        Self {
            speed: f32::from_bits(0x3C23_D70A),
            text_block: TextBlock::default(),
            position: [0.0; 3],
            max_width: -1,
        }
    }
}

/// The four kinds sharing graphics file reader's object registry. Cross-kind replacement retains
/// the latest identity, so a different kind cannot expose an earlier declaration of that name.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    Arrow(ArrowDefinition),
    AnimatedMapText(AnimatedMapTextDefinition),
    PdxMesh,
    PdxParticle,
}

/// An object declaration's common 3D fields and its kind's facts.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFacts {
    /// `name`, compared byte for byte in the separate object registry.
    pub name: Vec<u8>,
    /// `scale` (1).
    pub scale: f32,
    /// `cull_distance`, squared after every assignment, including failed conversions. Its default
    /// is the square of 430.
    pub cull_distance_squared: f32,
    pub kind: ObjectKind,
    pub refusals: Vec<Refusal>,
}

/// What the game keeps of one sprite declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteFacts {
    /// `name` (common sprite reader, string reader): the last assignment.
    pub name: Vec<u8>,
    /// `loadType`: `FRONTEND` 1, `BACKEND` 2, `INGAME` 3, byte for byte; anything else
    /// 0. Resource timing only.
    pub load_type: u8,
    pub kind: SpriteKind,
    /// What the game does not define in this declaration, by key.
    pub refusals: Vec<Refusal>,
}

/// The sprites one file declares, in declaration order, its font events, its last global
/// `textcolors` block, the global cursor offsets, and what its own level refuses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sprites {
    pub sprites: Vec<SpriteFacts>,
    /// The separate object registry's declarations, in reading order.
    pub objects: Vec<ObjectFacts>,
    /// Each `bitmapfont` and `bitmapfont_override`, in reading order.
    pub fonts: Vec<FontEvent>,
    /// The global table: each `textcolors` block clears all 256 entries and reads its own, so
    /// the last block wins.
    pub text_colors: Option<TextColors>,
    /// `cursor_offset` (kept in the graphics context) and `selection_offset`: The integer component
    /// reader into a zeroed `i64`, the last of each.
    pub cursor_offset: Option<[i32; 2]>,
    pub selection_offset: Option<[i32; 2]>,
    pub refusals: Vec<Refusal>,
}

impl Sprites {
    /// The latest declaration of `name`, compared byte for byte.
    pub fn get(&self, name: &[u8]) -> Option<&SpriteFacts> {
        self.sprites.iter().rev().find(|sprite| sprite.name == name)
    }

    /// The latest object of `name`, including a non-arrow declaration that replaced an arrow.
    pub fn object(&self, name: &[u8]) -> Option<&ObjectFacts> {
        self.objects.iter().rev().find(|object| object.name == name)
    }
}

/// Reads one `.gfx` file's declarations as the game reads them, with only the static keywords
/// registered ([`read_sprites_with`] adds the content's dynamic tokens).
pub fn read_sprites(bytes: &[u8]) -> Sprites {
    read_sprites_with(bytes, &DynamicTokens::default())
}

/// Reads one `.gfx` file's declarations as the game reads them, the lexer holding `dynamic`
/// ([`DynamicTokens::read`]).
pub fn read_sprites_with(bytes: &[u8], dynamic: &DynamicTokens) -> Sprites {
    let mut reader = Reader::new(
        crate::parser::native::lex(bytes, crate::parser::native::Mode::KeepMark),
        0,
    );
    reader.dynamic = dynamic.clone();
    let mut read = Sprites::default();
    while reader.next_key() {
        match keyword_of(&reader.key.clone()) {
            Some("spritetypes" | "objecttypes" | "bitmapfonts") => reader.gfx_block(&mut read),
            _ => reader.unexpected(),
        }
    }
    let mut refusals = std::mem::take(&mut reader.file);
    if let Some(poison) = reader.poison.take()
        && !refusals.contains(&poison)
    {
        refusals.push(poison);
    }
    read.refusals.extend(refusals);
    read
}

/// The three bootstrap files, read first in order and excluded by exact name from the main
/// pass.
pub const BOOTSTRAP: [&str; 3] = [
    "interface/load_screen.gfx",
    "interface/load_screen_font.gfx",
    "interface/core_bare_minimum.gfx",
];

/// Every `.gfx` path under `interface` in load order: existing bootstrap files requested by
/// literal lowercase path through the virtual file system, then `interface/**/*.gfx` excluding
/// those exact names. Visit each directory's files in byte order before its subdirectories,
/// depth first. Enumerated paths retain layer bytes ([`ResolvedLayers::listed_layer_paths`]).
pub fn gfx_order(layers: &ResolvedLayers) -> io::Result<Vec<Vec<u8>>> {
    let listed = crate::defines::load_order_paths(
        b"interface/",
        b".gfx",
        layers
            .listed_layer_paths(b"interface/")
            .iter()
            .map(Vec::as_slice),
    );
    // Bootstrap requests use folder lookup in any case and archive lookup by exact bytes. The
    // main pass excludes these exact names.
    let mut order = Vec::new();
    for path in BOOTSTRAP {
        if crate::content::open_request(layers, path.as_bytes())?.is_some() {
            order.push(path.as_bytes().to_vec());
        }
    }
    order.extend(listed.into_iter().filter(|path| {
        !BOOTSTRAP
            .iter()
            .any(|boot| boot.as_bytes() == path.as_slice())
    }));
    Ok(order)
}

/// Every declaration of the content stack's `interface/**/*.gfx`, in the game's load order
/// ([`gfx_order`]), each read by its path's bytes and its refusals prefixed with the path's text
/// (a label for messages). The lexer holds the stack's startup dynamic tokens
/// ([`DynamicTokens::read`]), registered before the interface is read.
pub fn read_layers(layers: &ResolvedLayers) -> io::Result<Vec<(String, Sprites)>> {
    read_paths(layers, gfx_order(layers)?)
}

/// The gameplay object's full `.gfx` order: the interface pass ([`gfx_order`]), then every
/// `gfx/**/*.gfx` in bytewise files-before-subdirectories order (graphics directory pass). All supported
/// recorded coverage policies hash these descriptors; reads retain the stack's path bytes.
pub fn object_gfx_order(layers: &ResolvedLayers) -> io::Result<Vec<Vec<u8>>> {
    let mut order = gfx_order(layers)?;
    order.extend(crate::defines::load_order_paths(
        b"gfx/",
        b".gfx",
        layers.listed_layer_paths(b"gfx/").iter().map(Vec::as_slice),
    ));
    Ok(order)
}

/// Reads the complete gameplay object registry's files using the same reader, including
/// declarations in `gfx` that replace interface objects. [`object_registry`] projects lookup.
pub fn read_object_layers(layers: &ResolvedLayers) -> io::Result<Vec<(String, Sprites)>> {
    read_paths(layers, object_gfx_order(layers)?)
}

fn read_paths(layers: &ResolvedLayers, order: Vec<Vec<u8>>) -> io::Result<Vec<(String, Sprites)>> {
    let dynamic = DynamicTokens::read(layers)?;
    let recorded = layers.without_overlays();
    let mut files = Vec::new();
    for bytes in order {
        let mut file = read_sprites_with(&layers.read_path(&bytes)?, &dynamic);
        let path = String::from_utf8_lossy(&bytes).into_owned();
        // A visual mod's file whose declarations are refused is read as the recording's own
        // file of that path when that reads whole, named.
        if let Some(refusal) = refusals_of(&file)
            && layers.supplied_by_visual_mod(&path)
            && let Some(recorded) = &recorded
            && let Ok(recorded_bytes) = recorded.read_path(&bytes)
        {
            let recorded_file = read_sprites_with(&recorded_bytes, &dynamic);
            if refusals_of(&recorded_file).is_none() {
                layers.note_overlay_refusal(format!("interface file {path} ({refusal})"));
                file = recorded_file;
            }
        }
        for refusal in file
            .refusals
            .iter_mut()
            .chain(
                file.sprites
                    .iter_mut()
                    .flat_map(|sprite| &mut sprite.refusals),
            )
            .chain(
                file.objects
                    .iter_mut()
                    .flat_map(|object| &mut object.refusals),
            )
        {
            refusal.path = if refusal.path.is_empty() {
                path.clone()
            } else {
                format!("{path}: {}", refusal.path)
            };
        }
        files.push((path, file));
    }
    Ok(files)
}

/// The first refusal anywhere in `file`: of the file itself, a sprite or an object.
fn refusals_of(file: &Sprites) -> Option<String> {
    file.refusals
        .iter()
        .chain(file.sprites.iter().flat_map(|sprite| &sprite.refusals))
        .chain(file.objects.iter().flat_map(|object| &object.refusals))
        .next()
        .map(ToString::to_string)
}

/// One sprite while its keys are read.
struct Declaring {
    facts: SpriteFacts,
    refused: BTreeMap<String, String>,
    /// The plain family's bytes past its named fields.
    image: Image,
}

impl Declaring {
    fn refuse(&mut self, key: &str, reason: String) {
        self.refused.entry(key.to_owned()).or_insert(reason);
    }
}

impl Reader<'_> {
    /// Read a `spriteTypes`, `objectTypes` or `bitmapfonts` block. A nonblock value causes the
    /// following tokens to be read as block keys, ending at `}` or EOF.
    fn gfx_block(&mut self, read: &mut Sprites) {
        while self.next_key() {
            let key = self.key.clone();
            let kind = match keyword_of(&key) {
                Some("spritetype" | "quadtexturesprite") => SpriteKind::Plain(Plain::default()),
                Some("frameanimatedspritetype") => SpriteKind::FrameAnimated {
                    plain: Plain::default(),
                    animation: FrameAnimation::default(),
                },
                Some("tilespritetype") => SpriteKind::Tile {
                    plain: Plain::default(),
                    size: [0; 2],
                },
                Some("maskedshieldtype") => SpriteKind::MaskedShield {
                    plain: Plain::default(),
                    shield: MaskedShield::default(),
                },
                Some("corneredtilespritetype") => SpriteKind::CorneredTile(CorneredTile {
                    frames: 1,
                    texture: Vec::new(),
                    size: [0; 2],
                    border: [0; 2],
                    effect_file: Vec::new(),
                    always_transparent: false,
                    tiling_center: false,
                    animation_rate: None,
                    looping: false,
                }),
                Some("progressbartype") => SpriteKind::Progress(Progress {
                    texture1: Vec::new(),
                    texture2: Vec::new(),
                    size: [0; 2],
                    color: [1.0; 4],
                    color_two: [1.0; 4],
                    effect_file: Vec::new(),
                    horizontal: true,
                    always_transparent: false,
                    steps: 100,
                }),
                Some("textspritetype") => SpriteKind::Text(TextSprite {
                    frames: 1,
                    texture: Vec::new(),
                    default_animation_time: 1.0,
                    effect_file: Vec::new(),
                    click_sound: Vec::new(),
                }),
                Some("resizeablespritetype") => SpriteKind::Resizeable(Resizeable {
                    frames: 1,
                    texture: Vec::new(),
                    size: [0; 2],
                }),
                Some("circularprogressbartype") => {
                    SpriteKind::CircularProgress(CircularProgress::default())
                }
                Some("piecharttype") => SpriteKind::PieChart(PieChart::default()),
                Some("linecharttype") => SpriteKind::LineChart(LineChart {
                    size: [0; 2],
                    effect_file: b"gfx/FX/linechart.lua".to_vec(),
                    half_width: 0.5,
                }),
                Some("arrowtype" | "animatedmaptext" | "pdxmesh" | "pdxparticle") => {
                    let word = keyword_of(&key).unwrap_or_default();
                    read.objects.push(self.object(word));
                    continue;
                }
                Some("bitmapfont") => {
                    let font = self.font();
                    read.fonts.push(FontEvent::Font(font));
                    continue;
                }
                Some("bitmapfont_override") => {
                    let replace = self.font_override();
                    read.fonts.push(FontEvent::Override(replace));
                    continue;
                }
                Some("textcolors") => {
                    // The global text color reader: the global table cleared, then read.
                    let mut table = TextColors::new();
                    let mut refusals = Vec::new();
                    self.text_colors(&mut table, &mut refusals);
                    read.refusals.extend(refusals);
                    read.text_colors = Some(table);
                    continue;
                }
                // Read integer components into a zeroed 8-byte local and beyond it
                // ([`OffsetFrame`]).
                Some(word @ ("cursor_offset" | "selection_offset")) => {
                    let mut written = Vec::new();
                    let result =
                        self.components(|index, component| written.push((index, component as u32)));
                    let mut frame = OffsetFrame { value: [0; 2] };
                    if let Err(reason) =
                        spill::spill(&mut frame, "integer component reader", 0, written)
                    {
                        record(&mut read.refusals, FILE, word, reason, Cause::Unread);
                    }
                    let value = frame.value;
                    if let Err(reason) = result {
                        self.poison(&mut read.refusals, FILE, word, &reason);
                    }
                    if word == "cursor_offset" {
                        read.cursor_offset = Some(value);
                    } else {
                        read.selection_offset = Some(value);
                    }
                    continue;
                }
                _ => {
                    self.unexpected();
                    continue;
                }
            };
            let facts = self.sprite(kind);
            read.sprites.push(facts);
        }
    }

    /// One sprite declaration: its reader's key loop over the tokens after the value.
    fn sprite(&mut self, kind: SpriteKind) -> SpriteFacts {
        let mut sprite = Declaring {
            facts: SpriteFacts {
                name: Vec::new(),
                load_type: 0,
                kind,
                refusals: Vec::new(),
            },
            refused: BTreeMap::new(),
            image: Image::default(),
        };
        while self.next_key() {
            let key = self.key.clone();
            let Some(word) = keyword_of(&key) else {
                self.unexpected();
                continue;
            };
            if !self.sprite_key(&mut sprite, word) && !self.base_key(&mut sprite, word) {
                self.unexpected();
            }
        }
        let (end, _) = allocation(&sprite.facts.kind);
        if let Some(plain) = sprite.facts.kind.plain_mut() {
            for (key, reason) in sprite.image.finish(plain, end) {
                sprite.refused.entry(key.to_owned()).or_insert(reason);
            }
        }
        let mut facts = sprite.facts;
        for (key, reason) in sprite.refused {
            record(&mut facts.refusals, FILE, &key, reason, Cause::Unread);
        }
        let name = String::from_utf8_lossy(&facts.name).into_owned();
        for refusal in &mut facts.refusals {
            refusal.path.clone_from(&name);
        }
        if let Some(poison) = &self.poison
            && !facts.refusals.contains(poison)
        {
            facts.refusals.push(poison.clone());
        }
        facts
    }

    /// The common sprite reader: the keys every sprite kind's reader hands on.
    fn base_key(&mut self, sprite: &mut Declaring, word: &str) -> bool {
        let text = self.value.text.clone();
        match word {
            "name" => sprite.facts.name = read_string(&text),
            // Compared byte for byte; anything else, a malformed value included, stores 0.
            "loadtype" => {
                sprite.facts.load_type = match read_string(&text).as_slice() {
                    b"FRONTEND" => 1,
                    b"BACKEND" => 2,
                    b"INGAME" => 3,
                    _ => 0,
                };
            }
            // Read into a local and dropped.
            "norefcount" => {}
            _ => return false,
        }
        true
    }

    /// The kind's own switch; `false` hands the key to [`Self::base_key`].
    fn sprite_key(&mut self, sprite: &mut Declaring, word: &str) -> bool {
        let text = self.value.text.clone();
        match &mut sprite.facts.kind {
            SpriteKind::FrameAnimated { animation, .. } => {
                // The frame animation reader's own keys, before the plain switch.
                match word {
                    "animation_rate_fps" => {
                        if let Some(value) = crate::crt::scan_f32(&text) {
                            animation.seconds_per_frame = value;
                        }
                        animation.seconds_per_frame = 1.0 / animation.seconds_per_frame;
                        return true;
                    }
                    "animation_rate_spf" => {
                        if let Some(value) = crate::crt::scan_f32(&text) {
                            animation.seconds_per_frame = value;
                        }
                        return true;
                    }
                    "looping" | "play_on_show" => {
                        if let Some(flag) = read_bool(&text) {
                            if word == "looping" {
                                animation.looping = flag;
                            } else {
                                animation.play_on_show = flag;
                            }
                        }
                        return true;
                    }
                    "pause_on_loop" => {
                        if let Some(value) = crate::crt::scan_f32(&text) {
                            animation.pause_on_loop = value;
                        }
                        return true;
                    }
                    _ => {}
                }
            }
            SpriteKind::MaskedShield { shield, .. } => match word {
                "texturefile1" => {
                    shield.texture1 = read_string(&text);
                    return true;
                }
                "texturefile2" => {
                    shield.texture2 = read_string(&text);
                    return true;
                }
                "effectfile" => {
                    shield.effect_file = read_string(&text);
                    return true;
                }
                "small" => {
                    if let Some(flag) = read_bool(&text) {
                        shield.small = flag;
                    }
                    return true;
                }
                _ => {}
            },
            SpriteKind::Tile { size, .. } if word == "size" => {
                let axes = self.int_point();
                store_axes(&mut sprite.refused, "size", axes, size);
                return true;
            }
            SpriteKind::CorneredTile(_) => return self.cornered_key(sprite, word, &text),
            SpriteKind::Progress(_) => return self.progress_key(sprite, word, &text),
            SpriteKind::Text(own) => {
                match word {
                    "noofframes" => {
                        own.frames = crate::crt::scan_u32(&text).stored().unwrap_or(own.frames)
                    }
                    "texturefile" => own.texture = read_string(&text),
                    "defaultanimationtime" => {
                        own.default_animation_time =
                            crate::crt::scan_f32(&text).unwrap_or(own.default_animation_time);
                    }
                    "effectfile" => own.effect_file = read_string(&text),
                    "clicksound" => own.click_sound = read_string(&text),
                    _ => return false,
                }
                return true;
            }
            SpriteKind::Resizeable(own) => {
                match word {
                    "noofframes" | "defaultanimationtime" => {
                        own.frames = crate::crt::scan_u32(&text).stored().unwrap_or(own.frames);
                    }
                    "texturefile" => own.texture = read_string(&text),
                    "size" => {
                        let axes = self.int_point();
                        store_axes(&mut sprite.refused, "size", axes, &mut own.size);
                    }
                    _ => return false,
                }
                return true;
            }
            SpriteKind::CircularProgress(own) => {
                match word {
                    "texturefile1" => own.texture1 = read_string(&text),
                    "texturefile2" => own.texture2 = read_string(&text),
                    "size" => own.size = crate::crt::scan_i32(&text).stored().unwrap_or(own.size),
                    "rotation" => {
                        if let Some(value) = crate::crt::scan_f32(&text) {
                            own.rotation = value;
                        }
                        own.rotation *= RADIANS;
                    }
                    "amount" => own.amount = crate::crt::scan_f32(&text).unwrap_or(own.amount),
                    _ => return false,
                }
                return true;
            }
            SpriteKind::PieChart(_) => {
                match word {
                    "size" => {
                        if let (Some(value), SpriteKind::PieChart(own)) =
                            (crate::crt::scan_i32(&text).stored(), &mut sprite.facts.kind)
                        {
                            own.size = Some(value);
                        }
                    }
                    "colors" => {
                        let colors = self.pie_colors();
                        if let SpriteKind::PieChart(own) = &mut sprite.facts.kind {
                            own.colors.extend(colors);
                        }
                    }
                    _ => return false,
                }
                return true;
            }
            SpriteKind::LineChart(own) => {
                match word {
                    "size" => {
                        let axes = self.int_point();
                        store_axes(&mut sprite.refused, "size", axes, &mut own.size);
                    }
                    "effectfile" => own.effect_file = read_string(&text),
                    "linewidth" => {
                        let value = crate::fixed5::script_fixed5(&text).unwrap_or(0);
                        own.half_width = value as f32 / 100_000.0 * 0.5;
                    }
                    _ => return false,
                }
                return true;
            }
            _ => {}
        }
        self.plain_key(sprite, word, &text)
    }

    /// Read the plain family's sprite fields.
    fn plain_key(&mut self, sprite: &mut Declaring, word: &str, text: &[u8]) -> bool {
        let Some(plain) = sprite.facts.kind.plain_mut() else {
            return false;
        };
        match word {
            "noofframes" => {
                plain.frames = crate::crt::scan_i32(text).stored().unwrap_or(plain.frames)
            }
            "texturefile" => plain.texture = read_string(text),
            // A kept record is appended to the animation records' vector ([`Records::append`]).
            "animation" => {
                let result = self.animation_block().and_then(|record| match record {
                    Some(record) => sprite.image.records.append(record),
                    None => Ok(()),
                });
                if let Err(reason) = result {
                    sprite.refuse("animation", reason);
                }
            }
            "defaultanimationtime" => {
                plain.default_animation_time =
                    crate::crt::scan_f32(text).unwrap_or(plain.default_animation_time);
            }
            "effectfile" => plain.effect_file = read_string(text),
            "transparencecheck" => {
                if let Some(flag) = read_bool(text) {
                    plain.transparence_check = flag;
                }
                plain.transparence_check_set = true;
            }
            "clicksound" | "masking_texture" => {
                let string = if word == "clicksound" {
                    &mut sprite.image.click
                } else {
                    &mut sprite.image.masking
                };
                match string
                    .assign(&read_string(text))
                    .and_then(|()| string.text())
                {
                    Ok(value) if word == "clicksound" => plain.click_sound = value,
                    Ok(value) => plain.masking_texture = value,
                    Err(reason) => sprite.refuse(word, reason),
                }
            }
            "allwaystransparent" | "alwaystransparent" => {
                plain.always_transparent = read_bool(text).unwrap_or(plain.always_transparent);
            }
            "can_be_lowres" => plain.can_be_lowres = read_bool(text).unwrap_or(plain.can_be_lowres),
            // A zeroed local: anything but `yes` stores 0.
            "framedirectiony" => {
                plain.frame_direction = i32::from(read_bool(text).unwrap_or(false));
            }
            "legacy_lazy_load" => {
                plain.legacy_lazy_load = read_bool(text).unwrap_or(plain.legacy_lazy_load);
            }
            "generate_mip_maps" => {
                plain.generate_mip_maps = read_bool(text).unwrap_or(plain.generate_mip_maps);
            }
            // The axis component reader into the anchor or the `dx_offset` flags, and on past them.
            "anchor" | "dx_offset" => {
                let (components, refused) = self.axis_components();
                if let Some(reason) = refused {
                    sprite.refuse(word, reason);
                    return true;
                }
                let (writer, start) = if word == "anchor" {
                    ("anchor", 0xE8)
                } else {
                    ("dx_offset", 0x1C8)
                };
                sprite.image.writer = writer;
                let mut bytes = SpriteBytes {
                    kind: &mut sprite.facts.kind,
                    image: &mut sprite.image,
                };
                let written = components
                    .into_iter()
                    .map(|(index, value)| (index, value.to_bits()));
                if let Err(reason) =
                    spill::spill(&mut bytes, "axis component reader", start, written)
                {
                    sprite.refuse(word, reason);
                }
            }
            _ => return false,
        }
        true
    }

    /// The cornered tile reader's own switch.
    fn cornered_key(&mut self, sprite: &mut Declaring, word: &str, text: &[u8]) -> bool {
        let SpriteKind::CorneredTile(own) = &mut sprite.facts.kind else {
            return false;
        };
        match word {
            "noofframes" => own.frames = crate::crt::scan_u32(text).stored().unwrap_or(own.frames),
            "texturefile" => own.texture = read_string(text),
            "effectfile" => own.effect_file = read_string(text),
            "size" | "bordersize" => {
                let mut point = if word == "size" { own.size } else { own.border };
                let axes = self.int_point();
                store_axes(&mut sprite.refused, word, axes, &mut point);
                if let SpriteKind::CorneredTile(own) = &mut sprite.facts.kind {
                    if word == "size" {
                        own.size = point;
                    } else {
                        own.border = point;
                    }
                }
            }
            "allwaystransparent" | "alwaystransparent" => {
                own.always_transparent = read_bool(text).unwrap_or(own.always_transparent);
            }
            "tilingcenter" => own.tiling_center = read_bool(text).unwrap_or(own.tiling_center),
            "looping" => own.looping = read_bool(text).unwrap_or(own.looping),
            // Stored as read.
            "animation_rate_fps" => {
                if let Some(value) = crate::crt::scan_f32(text) {
                    own.animation_rate = Some(value);
                }
            }
            // Stored, then inverted: an unwritten field stays unwritten.
            "animation_rate_spf" => {
                if let Some(value) = crate::crt::scan_f32(text) {
                    own.animation_rate = Some(value);
                }
                if let Some(rate) = &mut own.animation_rate {
                    *rate = 1.0 / *rate;
                }
            }
            _ => return false,
        }
        true
    }

    /// The progress bar reader's own keys.
    fn progress_key(&mut self, sprite: &mut Declaring, word: &str, text: &[u8]) -> bool {
        let SpriteKind::Progress(own) = &mut sprite.facts.kind else {
            return false;
        };
        match word {
            "texturefile1" => own.texture1 = read_string(text),
            "texturefile2" => own.texture2 = read_string(text),
            "effectfile" => own.effect_file = read_string(text),
            "size" => {
                let axes = self.int_point();
                if let SpriteKind::Progress(own) = &mut sprite.facts.kind {
                    let mut size = own.size;
                    store_axes(&mut sprite.refused, "size", axes, &mut size);
                    if let SpriteKind::Progress(own) = &mut sprite.facts.kind {
                        own.size = size;
                    }
                }
            }
            "steps" => own.steps = crate::crt::scan_i32(text).stored().unwrap_or(own.steps),
            "color" | "colortwo" => {
                let mut color = if word == "color" {
                    own.color
                } else {
                    own.color_two
                };
                self.color(&mut color);
                if let SpriteKind::Progress(own) = &mut sprite.facts.kind {
                    if word == "color" {
                        own.color = color;
                    } else {
                        own.color_two = color;
                    }
                }
            }
            "horizontal" => own.horizontal = read_bool(text).unwrap_or(own.horizontal),
            "allwaystransparent" | "alwaystransparent" => {
                own.always_transparent = read_bool(text).unwrap_or(own.always_transparent);
            }
            _ => return false,
        }
        true
    }

    /// A pie chart's `colors` reads without checking the value. Until a peeked `}` or EOF, read
    /// key/operator/value slots: `hsv` reads a color into one local, scales it and converts to
    /// RGB; `rgb` reads a color, while unknown keys skip a block. Append the local after each
    /// entry; it starts white and persists. Consume the closing `}`.
    fn pie_colors(&mut self) -> Vec<[f32; 4]> {
        let mut colors = Vec::new();
        let mut local = [1.0f32; 4];
        loop {
            if self.peek().ends_block() {
                self.read_resolved();
                return colors;
            }
            self.key_operator_value_logged();
            let key = self.key.clone();
            if key.is_keyword("hsv") {
                self.color(&mut local);
                let [h, s, v, a] = local;
                let [r, g, b] = hsv_to_rgb(h * 0.708_333_3, s * 2.55, v * 2.55);
                local = [r, g, b, a];
            } else if key.is_keyword("rgb") {
                self.color(&mut local);
            } else {
                self.unexpected();
            }
            colors.push(local);
        }
    }

    /// Read key/operator/value slots with extra diagnostics enabled.
    fn key_operator_value_logged(&mut self) {
        // The same walk as the key loop's.
        if !self.peek().ends_block() {
            self.next_key();
        }
    }

    /// Read animation tokens after the value into the 0x90-byte temporary record. Return
    /// `Ok(None)` when type stays unset and the record is dropped, retaining any
    /// component-written texture pointers; return `Err` for a write the game fails on. Default
    /// handling skips nothing.
    fn animation_block(&mut self) -> Result<Option<(AnimationData, RecordPointers)>, String> {
        let mut data = AnimationData::default();
        let mut pointers = RecordPointers::default();
        let mut frame = KeyLoopFrame::default();
        let mut failure = None;
        while self.next_key() {
            let key = self.key.clone();
            let text = self.value.text.clone();
            match keyword_of(&key) {
                Some("animationmaskfile") => data.mask_file = read_string(&text),
                Some("animationtexturefile") => data.texture_file = read_string(&text),
                Some("animationrotation") => {
                    data.rotation = crate::crt::scan_f32(&text).unwrap_or(0.0) * RADIANS;
                }
                Some("animationlooping") => {
                    data.looping = read_bool(&text).unwrap_or(data.looping);
                }
                Some("animationtime") => {
                    data.time = crate::crt::scan_f32(&text).unwrap_or(data.time)
                }
                Some("animationdelay") => {
                    data.delay = crate::crt::scan_f32(&text).unwrap_or(data.delay)
                }
                Some("animationtype") => {
                    // `tolower`, then an exact comparison; anything else keeps.
                    match read_string(&text).to_ascii_lowercase().as_slice() {
                        b"scrolling" => data.kind = 1,
                        b"rotating" | b"rotating_cw" => data.kind = 2,
                        b"rotating_ccw" => data.kind = 3,
                        b"pulsing" => data.kind = 4,
                        _ => {}
                    }
                }
                Some("animationblendmode") => {
                    match read_string(&text).to_ascii_lowercase().as_slice() {
                        b"add" => data.blend_mode = 0,
                        b"multiply" => data.blend_mode = 1,
                        b"overlay" => data.blend_mode = 2,
                        b"normal" => data.blend_mode = 3,
                        _ => {}
                    }
                }
                Some("animationtexturescale") => {
                    // Into a zeroed 8-byte local and beyond ([`ScaleFrame`]), then take
                    // reciprocals (1.0 for 0).
                    let (components, refused) = self.axis_components();
                    let mut scale = ScaleFrame {
                        local: [0.0; 2],
                        frame: &mut frame,
                    };
                    let written = components
                        .into_iter()
                        .map(|(index, value)| (index, value.to_bits()));
                    if let Err(reason) =
                        spill::spill(&mut scale, "axis component reader", 0, written)
                    {
                        failure.get_or_insert(format!("animationtexturescale {reason}"));
                    }
                    if let Some(reason) = refused {
                        failure.get_or_insert(reason);
                    }
                    data.texture_scale = scale
                        .local
                        .map(|value| if value == 0.0 { 1.0 } else { 1.0 / value });
                }
                Some("animationrotationoffset") => {
                    // Straight into the record from its type on ([`RecordBytes`]).
                    let (components, refused) = self.axis_components();
                    if let Some(reason) = refused {
                        failure.get_or_insert(reason);
                    }
                    let mut bytes = RecordBytes {
                        data: &mut data,
                        pointers: &mut pointers,
                    };
                    let written = components
                        .into_iter()
                        .map(|(index, value)| (index, value.to_bits()));
                    if let Err(reason) =
                        spill::spill(&mut bytes, "axis component reader", 0x60, written)
                    {
                        failure.get_or_insert(format!("animationrotationoffset {reason}"));
                    }
                }
                Some("animationframes") => {
                    let values = self.int_array();
                    if !values.is_empty() {
                        data.frames = values
                            .iter()
                            .fold(0, |mask, value| mask | 1u32 << (*value as u32 & 31));
                    }
                }
                Some("animationtriggerframes") => {
                    for value in self.int_array() {
                        data.trigger_frames |= 1u32 << (value as u32 & 31);
                    }
                }
                _ => {}
            }
        }
        if let Some(reason) = failure.or_else(|| frame.returned().err()) {
            return Err(reason);
        }
        Ok((data.kind != 0).then_some((data, pointers)))
    }

    /// The integer array reader: no check of the value; until a peeked `}` or the end
    /// (read), each token read and resolved is `%i` into a new element (0 when the scan stores
    /// nothing).
    fn int_array(&mut self) -> Vec<i32> {
        let mut values = Vec::new();
        loop {
            if self.peek().ends_block() {
                self.read();
                return values;
            }
            let token = self.read_resolved();
            values.push(crate::crt::scan_i32(&token.text).stored().unwrap_or(0));
        }
    }

    /// Read axis components only for a `{` value. Until `}` or EOF, each resolved token is a
    /// number scanned into the next component without a bound, or the next `x`, `y`, `z`, `w`
    /// label followed by `=` and a number. Other tokens report errors and skip blocks. Return
    /// each written component by index, and `Some` when a label is compared past the table of
    /// four.
    pub(super) fn axis_components(&mut self) -> (Vec<(usize, f32)>, Option<String>) {
        const LABELS: [&str; 4] = ["x", "y", "z", "w"];
        let mut components = Vec::new();
        if self.value.kind != TokenKind::Open {
            return (components, None);
        }
        let mut index = 0usize;
        loop {
            if self.peek().ends_block() {
                self.read();
                return (components, None);
            }
            let token = self.read_resolved();
            let number = if token.is_number() {
                Some(token)
            } else if index >= LABELS.len() {
                let reason = format!(
                    "holds {} after four components, which the axis component reader compares \
                     with a label read past its table of four",
                    display(&token)
                );
                self.poison(&mut Vec::new(), FILE, "", &reason);
                return (components, Some(reason));
            } else if token.is_keyword(LABELS[index]) {
                if self.read_resolved().kind == TokenKind::Equals {
                    let value = self.read_resolved();
                    if value.is_number() {
                        Some(value)
                    } else {
                        self.unexpected();
                        None
                    }
                } else {
                    self.unexpected();
                    None
                }
            } else {
                self.unexpected();
                None
            };
            if let Some(number) = number {
                if let Some(value) = crate::crt::scan_f32(&number.text) {
                    components.push((index, value));
                }
                index += 1;
            }
        }
    }

    /// An object-registry kind's token walk. Retain arrow and common base fields; other kinds'
    /// own fields keep their existing consumption.
    fn object(&mut self, word: &str) -> ObjectFacts {
        let kind = match word {
            "arrowtype" => ObjectKind::Arrow(ArrowDefinition::default()),
            "animatedmaptext" => ObjectKind::AnimatedMapText(AnimatedMapTextDefinition::default()),
            "pdxmesh" => ObjectKind::PdxMesh,
            "pdxparticle" => ObjectKind::PdxParticle,
            _ => unreachable!("only registered object kinds call this reader"),
        };
        let mut object = ObjectFacts {
            name: Vec::new(),
            scale: 1.0,
            cull_distance_squared: 430.0 * 430.0,
            kind,
            refusals: Vec::new(),
        };
        while self.next_key() {
            let key = keyword_of(&self.key.clone()).unwrap_or_default();
            if let ObjectKind::Arrow(arrow) = &mut object.kind
                && self.arrow_key(arrow, key)
            {
                continue;
            }
            let handled = match (word, key) {
                // The animated map text reader.
                ("animatedmaptext", "position") => {
                    let (components, failure) = self.axis_components();
                    if let ObjectKind::AnimatedMapText(text) = &mut object.kind {
                        for &(index, value) in &components {
                            match index {
                                0..=2 => text.position[index] = value,
                                3 => text.max_width = value.to_bits() as i32,
                                _ => {}
                            }
                        }
                    }
                    let failure = failure.or_else(|| {
                        components
                            .iter()
                            .find(|(index, _)| *index >= 4)
                            .map(|(index, _)| {
                                format!(
                                    "position component {} writes past the 0xF0-byte \
                                 animated map text object allocated by the graphics file reader \
                                 through the axis component reader",
                                    index + 1
                                )
                            })
                    });
                    if let Some(reason) = failure {
                        self.poison(&mut object.refusals, FILE, key, &reason);
                    }
                    true
                }
                ("animatedmaptext", "speed") => {
                    if let ObjectKind::AnimatedMapText(text) = &mut object.kind {
                        text.speed = crate::crt::scan_f32(&self.value.text).unwrap_or(text.speed);
                    }
                    true
                }
                ("animatedmaptext", "maxwidth") => {
                    if let ObjectKind::AnimatedMapText(text) = &mut object.kind {
                        text.max_width = crate::crt::scan_i32(&self.value.text)
                            .stored()
                            .unwrap_or(text.max_width);
                    }
                    true
                }
                ("animatedmaptext", "textblock") => {
                    if let ObjectKind::AnimatedMapText(text) = &mut object.kind {
                        self.read_text_block(&mut text.text_block);
                    }
                    true
                }
                // The mesh object reader.
                ("pdxmesh", "file" | "preload_textures") => true,
                ("pdxmesh", "animation" | "variant") => {
                    let names: &[&str] = if key == "animation" {
                        &["id", "type"]
                    } else {
                        &["pdxmesh", "weight"]
                    };
                    while self.next_key() {
                        let inner = keyword_of(&self.key.clone()).unwrap_or_default();
                        if !names.contains(&inner) {
                            self.unexpected();
                        }
                    }
                    true
                }
                ("pdxmesh", "meshsettings") => {
                    self.mesh_settings();
                    true
                }
                // The particle definition lookup.
                ("pdxparticle", "type") => true,
                _ => false,
            };
            if handled {
                continue;
            }
            // The 3D object reader: failed scalar conversions retain the destination;
            // cull_distance squares that retained value as well.
            match key {
                "name" => object.name = read_string(&self.value.text),
                "scale" => {
                    object.scale = crate::crt::scan_f32(&self.value.text).unwrap_or(object.scale);
                }
                "cull_distance" => {
                    let distance = crate::crt::scan_f32(&self.value.text)
                        .unwrap_or(object.cull_distance_squared);
                    object.cull_distance_squared = distance * distance;
                }
                _ => self.unexpected(),
            }
        }
        if let Some(poison) = &self.poison
            && !object.refusals.contains(poison)
        {
            object.refusals.push(poison.clone());
        }
        let name = String::from_utf8_lossy(&object.name).into_owned();
        for refusal in &mut object.refusals {
            refusal.path.clone_from(&name);
        }
        object
    }

    /// Read arrow fields. Scalars use shared CRT conversion and consume no further token; both
    /// colors read the lexer's current token.
    fn arrow_key(&mut self, arrow: &mut ArrowDefinition, key: &str) -> bool {
        let text = &self.value.text;
        match key {
            "size" => arrow.size = crate::crt::scan_f32(text).unwrap_or(arrow.size),
            "endat" => arrow.end_at = crate::crt::scan_f32(text).unwrap_or(arrow.end_at),
            "height" => arrow.height = crate::crt::scan_f32(text).or(arrow.height),
            "type" => arrow.arrow_type = crate::crt::scan_i32(text).stored().or(arrow.arrow_type),
            "heading" => arrow.heading = crate::crt::scan_f32(text).unwrap_or(arrow.heading),
            "texture" => arrow.texture = read_string(text),
            "normal" => arrow.normal = read_string(text),
            "specular" => arrow.specular = read_string(text),
            "overlay" => arrow.overlay = read_string(text),
            "effect" => arrow.effect = read_string(text),
            "color" => self.color(&mut arrow.color),
            "colortwo" => self.color(&mut arrow.color_two),
            _ => return false,
        }
        true
    }

    /// Read mesh data by token id ([`keywords::class`]): `name` (0x1B), `shader` (0x1B0),
    /// `index` (0x20C, `%i`) and `shader_file` (0x2A6) take their value. String keys (0xF:
    /// quoted or unregistered) are texture overrides: store `texture_diffuse`, report other
    /// names, and skip nothing. A `{` value therefore leaves its keys to be read as settings,
    /// and its `}` ends them. Other ids (keywords, dynamic tokens, numbers) are unexpected and
    /// skip a block.
    fn mesh_settings(&mut self) {
        while self.next_key() {
            let key = self.key.clone();
            match keywords::class(&key, &self.dynamic) {
                Class::Static("name" | "shader" | "index" | "shader_file") | Class::String => {}
                Class::Static(_) | Class::Dynamic | Class::Number => self.unexpected(),
            }
        }
    }

    /// Read text colors into `table` from the token after the value until the current token is
    /// `}`. Each entry takes a code token's first byte, a token as `=`, and a color. Retain the
    /// first color of each code, logging later ones; code 0 always looks free. Ignored read
    /// results leave the frozen token visible at EOF: `}` ends the walk and other tokens loop
    /// forever. Code bytes from 0x80 index 192 GiB past the table.
    pub(super) fn text_colors(&mut self, table: &mut TextColors, refusals: &mut Vec<Refusal>) {
        self.read_resolved();
        loop {
            if self.current.kind == TokenKind::Close {
                return;
            }
            if self.at_end() {
                self.poison(
                    refusals,
                    FILE,
                    "textcolors",
                    "ends with the file on a token that is not `}`, which the text color reader \
                     reads forever",
                );
                return;
            }
            let code = self.current.text.first().copied().unwrap_or(0);
            self.read_resolved();
            self.read_resolved();
            let mut color = [1.0; 4];
            self.color(&mut color);
            self.read_resolved();
            if code >= 0x80 {
                record(
                    refusals,
                    FILE,
                    "textcolors",
                    format!(
                        "has the code byte {code:#x}, which the text color reader sign-extends \
                         into an index 192 GiB past the table: an invalid read"
                    ),
                    Cause::Unread,
                );
                continue;
            }
            if code == 0 || !table.contains_key(&code) {
                table.insert(code, color);
            }
        }
    }
}

/// Stores what integer point reader wrote into the field, refusing it for an undefined axis.
fn store_axes(
    refused: &mut BTreeMap<String, String>,
    key: &str,
    axes: [Axis; 2],
    target: &mut [i32; 2],
) {
    for (target, axis) in target.iter_mut().zip(axes) {
        match axis {
            Axis::Untouched => {}
            Axis::Value(value) => *target = value,
            Axis::Undefined => {
                refused.entry(key.to_owned()).or_insert_with(|| {
                    "scans nothing of its first value, so the integer point reader stores its \
                     uninitialised local: an undefined read"
                        .into()
                });
                return;
            }
        }
    }
}

/// The plain sprite's animation record vector (data, capacity, count and allocator): the kept
/// `animation` records, each with the texture pointers a component wrote into it.
type Records = Vector<(AnimationData, RecordPointers)>;

/// Where the records' vector sits and who uses it.
const RECORDS: VectorSite = VectorSite {
    what: "the animation records' vector",
    append: "sprite reader",
    readers: "texture release, effect program selection and sprite instance constructor",
    destructor: "sprite destructor",
};

/// A plain-family sprite's bytes past the fields its keys name, as the component readers reach
/// them (`super::spill`): the animation records, the two strings, and what a component wrote
/// over fields that decide the game's later reads.
#[derive(Clone, Debug)]
struct Image {
    records: Records,
    /// `masking_texture` and `clicksound`.
    masking: CxxString,
    click: CxxString,
    /// The two animation records' technique handles (-1).
    handles: [u32; 8],
    /// The base and masking texture pointers (null), with the key that last wrote each.
    base_texture: [u32; 2],
    mask_texture: [u32; 2],
    /// The key whose components last wrote a field [`Image::finish`] checks.
    writer: &'static str,
}

impl Default for Image {
    fn default() -> Self {
        Self {
            records: Vector::new(RECORDS),
            masking: CxxString::default(),
            click: CxxString::default(),
            handles: [u32::MAX; 8],
            base_texture: [0; 2],
            mask_texture: [0; 2],
            writer: "anchor",
        }
    }
}

impl Image {
    /// What the declaration leaves once read: the records the readers see, the masking texture
    /// the resource initialiser loads, and the failures of fields a component left for a later
    /// read.
    fn finish(&mut self, plain: &mut Plain, end: usize) -> Vec<(&'static str, String)> {
        let mut failures = Vec::new();
        let mut pointers = Vec::new();
        match self.records.visible() {
            Ok(records) => {
                (plain.animations, pointers) = records.into_iter().unzip();
            }
            Err(reason) => failures.push(("animation", reason)),
        }
        match self.masking.text_within(end - 0x180) {
            Ok(text) => plain.masking_texture = text,
            // The animation initialization loads it only when its size is not 0.
            Err(reason) => failures.push((
                self.writer,
                format!(
                    "leaves masking_texture unreadable, which the animation \
                     initialization loads: {reason}"
                ),
            )),
        }
        // Read only when the sprite is clicked (a map icon takes no pointer: census K); its
        // destructor reads only the capacity and pointer the writes already checked.
        if let Ok(text) = self.click.text_within(end - 0x1A8) {
            plain.click_sound = text;
        }
        let lazy = plain.legacy_lazy_load;
        if lazy
            && !plain.animations.is_empty()
            && self.handles.iter().any(|handle| *handle != u32::MAX)
        {
            failures.push((
                self.writer,
                "wrote the animation records' technique handles, which the \
                 animation initialization rewrites only for an eagerly loaded sprite and effect \
                 program selection resolves by the renderer's registration index, which the \
                 replay does not hold"
                    .into(),
            ));
        }
        // An eagerly loaded sprite's records bind their texture pointers
        // (effect program selection through animation UV scaling), which
        // The animation initialization replaces only for a file that is set; a lazily
        // loaded sprite's records never resolve an effect, so nothing binds them.
        for (record, written) in plain.animations.iter().zip(&pointers) {
            if lazy {
                break;
            }
            if written.mask && record.mask_file.is_empty()
                || written.texture && record.texture_file.is_empty()
            {
                failures.push((
                    "animation",
                    "has a texture pointer animationrotationoffset wrote (mask or texture) for a \
                     file that is not set, which the animation initialization does not replace \
                     and effect program selection binds as a texture"
                        .into(),
                ));
                break;
            }
        }
        if self.base_texture != [0; 2] && (lazy || plain.texture.is_empty()) {
            failures.push((
                self.writer,
                "wrote the base texture pointer, which the animation initialization \
                 replaces only for an eagerly loaded sprite with a textureFile, so the sprite \
                 draw binds the written bits as a texture"
                    .into(),
            ));
        }
        if self.mask_texture != [0; 2] && self.masking.is_empty() {
            failures.push((
                self.writer,
                "wrote the masking texture pointer, which the animation initialization \
                 replaces only when masking_texture is set, so the sprite draw binds the written \
                 bits as the mask"
                    .into(),
            ));
        }
        failures
    }
}

/// Which of an animation record's texture pointers (mask, then texture, both null in sprite
/// reader's record) a component wrote with other bits.
#[derive(Clone, Copy, Debug, Default)]
struct RecordPointers {
    mask: bool,
    texture: bool,
}

/// The temporary animation record's 0x90 bytes from `animationrotationoffset` through the saved
/// registers immediately after the record.
struct RecordBytes<'a> {
    data: &'a mut AnimationData,
    pointers: &'a mut RecordPointers,
}

impl Layout for RecordBytes<'_> {
    fn end(&self) -> (usize, String) {
        (
            0x90,
            "past the record, the saved register after the sprite reader's record: the caller's \
             registers are undefined"
                .into(),
        )
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let data = &mut *self.data;
        match offset {
            0x60 | 0x64 => data.rotation_offset[(offset - 0x60) / 4] = f32::from_bits(bits),
            0x68 => data.kind = bits as i32,
            0x6C => data.blend_mode = bits as i32,
            0x70 => data.frames = bits,
            0x74 => data.trigger_frames = bits,
            0x78 | 0x7C => self.pointers.mask |= bits != 0,
            0x80 | 0x84 => self.pointers.texture |= bits != 0,
            _ => data.texture_size[(offset - 0x88) / 4] = bits as i32,
        }
        Ok(())
    }
}

/// The persistent key-loop frame while reading an animation, as texture-scale components change it.
/// Duplicate checking is disabled for animation records, so the seen-key buffer is unused during
/// reading. On return, a nonnull buffer pointer (the inline buffer's stack address while intact) is
/// freed through the frame's allocator, with its virtual-function table.
#[derive(Debug, Default)]
struct KeyLoopFrame {
    /// The buffer pointer's halves a component wrote; `None` keeps the stack address.
    buffer: [Option<u32>; 2],
    /// Whether a component wrote the allocator's address or its vtable.
    allocator: bool,
}

impl KeyLoopFrame {
    /// What the block's return path meets: a null pointer skips the free; an intact one frees
    /// through an intact allocator; anything else is a stack or load address the replay does
    /// not hold.
    fn returned(&self) -> Result<(), String> {
        match self.buffer {
            [Some(0), Some(0)] => Ok(()),
            [None, None] if !self.allocator => Ok(()),
            [None, None] => Err(
                "animationtexturescale wrote the key loop's allocator (the persistent key loop's \
             allocator address or vtable), through which its return path frees its \
             buffer: a stack or load address the replay does not hold"
                    .into(),
            ),
            _ => Err(
                "animationtexturescale wrote the key loop's buffer pointer (the persistent key \
             loop's), which its return path frees when it is not null: bits that are not the \
             inline buffer's stack address, which the replay does not hold"
                    .into(),
            ),
        }
    }
}

/// The animation texture-scale reader's zeroed 8-byte local in the enclosing key-loop frame.
/// Components spill over that frame ([`KeyLoopFrame`]) through its saved floating-point register.
struct ScaleFrame<'a> {
    local: [f32; 2],
    frame: &'a mut KeyLoopFrame,
}

impl Layout for ScaleFrame<'_> {
    fn end(&self) -> (usize, String) {
        (
            0x238,
            "persistent key loop's saved XMM6 and then its saved registers and return address: \
             the caller's registers are undefined"
                .into(),
        )
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        match offset {
            0 | 4 => self.local[offset / 4] = f32::from_bits(bits),
            // The buffer pointer.
            0x18 | 0x1C => self.frame.buffer[(offset - 0x18) / 4] = Some(bits),
            // The allocator's address and vtable.
            0x28..=0x34 => self.frame.allocator = true,
            // The rest: an outgoing argument written before use; an unused slot; the buffer
            // capacity and count, read only by disabled duplicate checking (return rewrites count);
            // the inline entries; the fallback allocator, read only for pointers outside that
            // buffer; token copies rewritten before each key reads them, and duplicate-path log
            // objects.
            _ => {}
        }
        Ok(())
    }
}

/// The graphics file reader's frame from its zeroed 8-byte offset local through saved registers.
/// The live sub-reader object pointer follows the local; remaining slots are unused here or belong
/// to a font override whose lifetime has ended.
struct OffsetFrame {
    value: [i32; 2],
}

impl Layout for OffsetFrame {
    fn end(&self) -> (usize, String) {
        (
            0x80,
            "the saved R14, RDI and RBP and the return address, which the epilogue restores: the \
             caller's registers and return are undefined"
                .into(),
        )
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        match offset {
            0 | 4 => self.value[offset / 4] = bits as i32,
            0x10 | 0x14 if bits != 0 => {
                return Err(
                    "the graphics file reader's out-pointer, which it reads after its field \
                     dispatch and, when it is not null, calls through: bits that are no object"
                        .into(),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

/// A plain-family sprite and its subtype tail as component readers write into them.
struct SpriteBytes<'a> {
    kind: &'a mut SpriteKind,
    image: &'a mut Image,
}

/// A plain-family allocation's size and the kind of reader creating it.
fn allocation(kind: &SpriteKind) -> (usize, &'static str) {
    match kind {
        SpriteKind::FrameAnimated { .. } | SpriteKind::Tile { .. } => {
            (0x200, "graphics file reader")
        }
        SpriteKind::MaskedShield { .. } => (0x2C0, "graphics resource reader"),
        _ => (0x1F0, "graphics file reader"),
    }
}

impl Layout for SpriteBytes<'_> {
    fn end(&self) -> (usize, String) {
        let (size, allocated) = allocation(self.kind);
        spill::heap_end(size, allocated)
    }

    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let float = f32::from_bits(bits);
        let bytes = bits.to_le_bytes();
        let image = &mut *self.image;
        match (&mut *self.kind, offset) {
            (SpriteKind::FrameAnimated { animation, .. }, 0x1F0..) => {
                match offset {
                    0x1F0 => animation.seconds_per_frame = float,
                    // `looping` and `play_on_show`; the other two bytes are padding.
                    0x1FC => {
                        animation.looping = bytes[0] != 0;
                        animation.play_on_show = bytes[1] != 0;
                    }
                    0x1F8 => animation.pause_on_loop = float,
                    // The upper half of the constructor's 8-byte store over seconds per frame: no
                    // reader (frame animation draw reads only the lower half).
                    _ => {}
                }
                return Ok(());
            }
            (SpriteKind::Tile { size, .. }, 0x1F0..) => {
                // `size`; the next eight bytes no constructor writes and no reader reads.
                if let Some(axis) = [0x1F0, 0x1F4].iter().position(|at| *at == offset) {
                    size[axis] = bits as i32;
                }
                return Ok(());
            }
            (SpriteKind::MaskedShield { .. }, 0x1F0..) => {
                return Err(
                    "the masked sprite type's graphics pointer, to which the flag atlas \
                     selection adds the flag atlas's offset: a heap address the replay does not \
                     hold"
                        .into(),
                );
            }
            _ => {}
        }
        let Some(plain) = self.kind.plain_mut() else {
            return Ok(());
        };
        match offset {
            0xE8 | 0xEC => plain.anchor[(offset - 0xE8) / 4] = float,
            // The geometry scale: sprite vertex buffer creation writes -1.0 before
            // any draw, and every draw (sprite draw) rewrites it from the world
            // matrix before geometry construction reads it.
            0xF0 => {}
            0xF4 => plain.stretch = float,
            0xF8 => plain.v_scale = float,
            // The frame index sprite draw writes before frame offset calculation reads it.
            0xFC => {}
            0x100 | 0x104 => plain.frame_step[(offset - 0x100) / 4] = float,
            // The geometry-dirty byte vertex buffer creation sets before any draw; its
            // other three bytes are padding.
            0x108 => {}
            0x10C => plain.frame_direction = bits as i32,
            // The flip, then `alwaystransparent`; the constructor writes only these two (the other
            // two bytes are padding).
            0x110 => {
                plain.flip_u = bytes[0] != 0;
                plain.always_transparent = bytes[1] != 0;
            }
            0x114 => {}
            0x118..=0x12C => image.records.write(offset - 0x118, bits, image.writer)?,
            0x130..=0x14C => image.handles[(offset - 0x130) / 4] = bits,
            // The base effect's and the text effect's technique handles, which the animation
            // initialization writes (base technique selection, then text technique selection)
            // before any draw.
            0x150..=0x16C => {}
            0x170 | 0x174 => image.base_texture[(offset - 0x170) / 4] = bits,
            0x178 | 0x17C => image.mask_texture[(offset - 0x178) / 4] = bits,
            0x180..=0x19C => image.masking.write(offset - 0x180, bits)?,
            0x1A0 => plain.texture_id = bits as i32,
            // The drawable's opacity, which sprite instance draw and
            // The frame animation draw write before the type's draw and reset to
            // 1.0 after.
            0x1A4 => {}
            0x1A8..=0x1C4 => image.click.write(offset - 0x1A8, bits)?,
            0x1C8 | 0x1CC => plain.dx_offset[(offset - 0x1C8) / 4] = float,
            0x1D0 => {
                plain.transparence_check = bytes[0] != 0;
                plain.transparence_check_set = bytes[1] != 0;
                plain.can_be_lowres = bytes[2] != 0;
                plain.legacy_lazy_load = bytes[3] != 0;
            }
            // The first byte only decides whether effect program selection logs once; the second is
            // `generate_mip_maps`; the last two are padding.
            0x1D4 => plain.generate_mip_maps = bytes[1] != 0,
            0x1D8 | 0x1DC if bits != 0 => {
                return Err(
                    "the second vector's data pointer, which the sprite destructor frees \
                 through its allocator when it is not null: bits that are no block"
                        .into(),
                );
            }
            // That vector's capacity, count and allocator: the destructor reads them only while the
            // data pointer is not null, and nothing appends to it.
            _ => {}
        }
        Ok(())
    }
}

/// The HSV-to-RGB conversion in place on the scaled components.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    if s == 0.0 {
        return [v, v, v];
    }
    let i = (h * 6.0).floor().min(5.0);
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Every declaration's refusals and every file's, with their paths.
pub fn refusals(files: &[(String, Sprites)]) -> Vec<&Refusal> {
    files
        .iter()
        .flat_map(|(_, file)| {
            file.refusals
                .iter()
                .chain(file.sprites.iter().flat_map(|sprite| &sprite.refusals))
                .chain(file.objects.iter().flat_map(|object| &object.refusals))
        })
        .collect()
}

/// The latest declaration of every name over files in load order.
pub fn registry(files: &[(String, Sprites)]) -> BTreeMap<Vec<u8>, &SpriteFacts> {
    let mut registry = BTreeMap::new();
    for (_, file) in files {
        for sprite in &file.sprites {
            registry.insert(sprite.name.clone(), sprite);
        }
    }
    registry
}

/// The latest declaration of every object name over files in load order, across all four
/// object kinds. The sprite and font registries are independent.
pub fn object_registry(files: &[(String, Sprites)]) -> BTreeMap<Vec<u8>, &ObjectFacts> {
    let mut registry = BTreeMap::new();
    for (_, file) in files {
        for object in &file.objects {
            registry.insert(object.name.clone(), object);
        }
    }
    registry
}

#[cfg(test)]
mod tests;
