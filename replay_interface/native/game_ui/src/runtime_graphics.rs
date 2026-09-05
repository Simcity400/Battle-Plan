
use crate::assets::AssetResolver;
use crate::authored_role_index::{AuthenticatedGuiRoleIndex, StrictAuthoredPanelAdmission};
use crate::dds::RgbaImage;
use crate::gfx::SpriteDefinition;
use crate::widget_visual_state::{AuthenticatedWidgetGraphicsState, ProgressEffect, ProgressFill};
use crate::window_manager_law::{
    AuthenticatedReplayFrameAuthority, AuthenticatedReplayPanelLease, ReplayOwnedWindowManager,
};
use replay_core::ValidatedContentAdmission;
use snapshot::sha256::Sha256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonVisualState {
    Up,
    Down,
    Over,
    Disable,
}

impl ButtonVisualState {
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Up),
            1 => Some(Self::Down),
            2 => Some(Self::Over),
            3 => Some(Self::Disable),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSelection {
    pub one_based_frame: u32,
    pub zero_based_index: usize,
}

pub fn select_one_based_frame(frame: u32, frame_count: usize) -> Option<FrameSelection> {
    if frame == 0 || frame as usize > frame_count {
        return None;
    }
    Some(FrameSelection {
        one_based_frame: frame,
        zero_based_index: frame as usize - 1,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbaF32(pub [f32; 4]);

impl RgbaF32 {
    pub fn finite(self) -> bool {
        self.0.iter().all(|value| value.is_finite())
    }
}

fn saturate(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}

pub fn lerp_rgba(first: RgbaF32, second: RgbaF32, factor: f32) -> RgbaF32 {
    RgbaF32(std::array::from_fn(|index| {
        lerp(first.0[index], second.0[index], factor)
    }))
}

pub fn modulate_rgba(pixel: RgbaF32, color: RgbaF32) -> RgbaF32 {
    RgbaF32(std::array::from_fn(|index| pixel.0[index] * color.0[index]))
}

pub fn shade_static_button(base: RgbaF32, color: RgbaF32) -> RgbaF32 {
    modulate_rgba(base, color)
}

pub fn apply_opacity(pixel: RgbaF32, opacity: f32) -> RgbaF32 {
    let mut out = pixel;
    out.0[3] *= opacity;
    out
}

fn luminance(pixel: RgbaF32) -> f32 {
    pixel.0[0] * 0.212671 + pixel.0[1] * 0.715160 + pixel.0[2] * 0.072169
}

pub fn shade_buttonstate(
    base_after_animation_and_mask: RgbaF32,
    color: RgbaF32,
    state: ButtonVisualState,
    time: f32,
    animation_time: f32,
) -> RgbaF32 {
    let mut out = base_after_animation_and_mask;
    if state == ButtonVisualState::Disable {
        let grey = luminance(out);
        out.0[0] = grey;
        out.0[1] = grey;
        out.0[2] = grey;
    }
    out = modulate_rgba(out, color);
    let speed = match state {
        ButtonVisualState::Down => Some((16.0, -1.0)),
        ButtonVisualState::Over => Some((4.0, 1.0)),
        _ => None,
    };
    if let Some((speed, sign)) = speed {
        let mut transition = 0.9 - saturate((time - animation_time) * speed);
        transition *= transition;
        transition = 0.9 * 0.9 - transition;
        let mix = 0.15 * transition;
        for channel in 0..3 {
            out.0[channel] += sign * (0.5 + out.0[channel]) * mix;
        }
    }
    out
}

fn shade_buttonstate_no_down_effect(
    base: RgbaF32,
    color: RgbaF32,
    state: ButtonVisualState,
) -> RgbaF32 {
    let mut out = base;
    match state {
        ButtonVisualState::Disable => {
            let grey = luminance(out);
            out.0[0] = grey;
            out.0[1] = grey;
            out.0[2] = grey;
        }
        ButtonVisualState::Over => {
            for channel in 0..3 {
                out.0[channel] += 0.1;
            }
        }
        ButtonVisualState::Up | ButtonVisualState::Down => {}
    }
    modulate_rgba(out, color)
}

fn shade_buttonstate_no_down_or_disable_effect(
    base: RgbaF32,
    color: RgbaF32,
    state: ButtonVisualState,
) -> RgbaF32 {
    let mut out = base;
    if state == ButtonVisualState::Over {
        for channel in 0..3 {
            out.0[channel] += 0.1;
        }
    }
    modulate_rgba(out, color)
}

fn shade_buttonstate_only_disable(
    base: RgbaF32,
    color: RgbaF32,
    state: ButtonVisualState,
) -> RgbaF32 {
    let mut out = base;
    if state == ButtonVisualState::Disable {
        let grey = luminance(out);
        out.0[0] = grey;
        out.0[1] = grey;
        out.0[2] = grey;
    }
    modulate_rgba(out, color)
}

pub fn shade_buttonstate_blendframes(
    current: RgbaF32,
    next: RgbaF32,
    animation_time: f32,
    color: RgbaF32,
    state: ButtonVisualState,
) -> RgbaF32 {
    let mut out = lerp_rgba(current, next, animation_time);
    match state {
        ButtonVisualState::Up => modulate_rgba(out, color),
        ButtonVisualState::Disable => {
            let grey = luminance(out);
            out.0[0] = grey;
            out.0[1] = grey;
            out.0[2] = grey;
            modulate_rgba(out, color)
        }
        ButtonVisualState::Down | ButtonVisualState::Over => {
            let sign = if state == ButtonVisualState::Down {
                -1.0
            } else {
                1.0
            };
            for channel in 0..3 {
                out.0[channel] += sign * (0.5 + out.0[channel]) * 0.15;
            }
            out
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteAnimationBlendMode {
    Add,
    Multiply,
    Overlay,
    Normal,
}

pub fn blend_sprite_animation(
    dest: RgbaF32,
    source: RgbaF32,
    mode: SpriteAnimationBlendMode,
) -> RgbaF32 {
    match mode {
        SpriteAnimationBlendMode::Add => RgbaF32([
            lerp(dest.0[0], dest.0[0] + source.0[0], source.0[3]),
            lerp(dest.0[1], dest.0[1] + source.0[1], source.0[3]),
            lerp(dest.0[2], dest.0[2] + source.0[2], source.0[3]),
            dest.0[3].max(source.0[3]),
        ]),
        SpriteAnimationBlendMode::Multiply => {
            RgbaF32(std::array::from_fn(|index| dest.0[index] * source.0[index]))
        }
        SpriteAnimationBlendMode::Overlay => {
            let rgb: [f32; 3] = std::array::from_fn(|index| {
                if dest.0[index] >= 0.5 {
                    1.0 - 2.0 * (1.0 - dest.0[index]) * (1.0 - source.0[index])
                } else {
                    dest.0[index] * source.0[index] * 2.0
                }
            });
            RgbaF32([rgb[0], rgb[1], rgb[2], source.0[3]])
        }
        SpriteAnimationBlendMode::Normal => RgbaF32([
            lerp(dest.0[0] * dest.0[3], source.0[0], source.0[3]),
            lerp(dest.0[1] * dest.0[3], source.0[1], source.0[3]),
            lerp(dest.0[2] * dest.0[3], source.0[2], source.0[3]),
            source.0[3].max(dest.0[3]),
        ]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ClipRect {
    pub fn is_nonempty(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let value = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        value.is_nonempty().then_some(value)
    }

    pub fn contains_pixel(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

pub fn blend_src_alpha_inv_src_alpha(dest: RgbaF32, source: RgbaF32) -> RgbaF32 {
    let source_factor = source.0[3];
    let dest_factor = 1.0 - source_factor;
    RgbaF32([
        source.0[0] * source_factor + dest.0[0] * dest_factor,
        source.0[1] * source_factor + dest.0[1] * dest_factor,
        source.0[2] * source_factor + dest.0[2] * dest_factor,
        source.0[3],
    ])
}

pub fn blend_text_src_alpha_inv_src_alpha_max_alpha(dest: RgbaF32, source: RgbaF32) -> RgbaF32 {
    let source_factor = source.0[3];
    let dest_factor = 1.0 - source_factor;
    RgbaF32([
        source.0[0] * source_factor + dest.0[0] * dest_factor,
        source.0[1] * source_factor + dest.0[1] * dest_factor,
        source.0[2] * source_factor + dest.0[2] * dest_factor,
        source.0[3].max(dest.0[3]),
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingPath {
    ButtonState,
    ButtonStateBlendFrames,
    StaticButton,
    ProducerResolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticatedUiShaderLaw {
    ButtonState,
    ButtonStateNoDownEffect,
    ButtonStateNoDownOrDisableEffect,
    ButtonStateOnlyDisable,
    ButtonStateBlendFrames,
    StaticButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticatedProgressShaderLaw {
    Linear,
    StartEnd,
    Reverse,
    MinMax,
    Radial,
    Circular,
}

impl AuthenticatedUiShaderLaw {
    fn shading_path(self) -> ShadingPath {
        match self {
            Self::ButtonState
            | Self::ButtonStateNoDownEffect
            | Self::ButtonStateNoDownOrDisableEffect
            | Self::ButtonStateOnlyDisable => ShadingPath::ButtonState,
            Self::ButtonStateBlendFrames => ShadingPath::ButtonStateBlendFrames,
            Self::StaticButton => ShadingPath::StaticButton,
        }
    }
}

const BUTTONSTATE_SHADER_SHA256: &str =
    "1eddb371702289f4cc9d9d565aef21cf77c9261bd4324194295eeea6309f413a";
const BUTTONSTATE_SHADER_BYTES: u64 = 4_867;
const BUTTONSTATE_NO_DOWN_EFFECT_SHADER_SHA256: &str =
    "e44bcaaaf2542bf7ab1c23d9670be72cfcf5cd3d51d3197f8f4836035a74ace3";
const BUTTONSTATE_NO_DOWN_EFFECT_SHADER_BYTES: u64 = 2_291;
const BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_SHA256: &str =
    "641f9245ce6eda5135a9c84ee7723e1e3d268b5c16d2b0bd14e5e85cca432075";
const BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_BYTES: u64 = 1_706;
const BUTTONSTATE_ONLY_DISABLE_SHADER_SHA256: &str =
    "8c43262388847cc5b16d44da800f0d2d1eb8b900c126dd78c7cc57c5ab483002";
const BUTTONSTATE_ONLY_DISABLE_SHADER_BYTES: u64 = 2_510;
const BUTTONSTATE_BLEND_FRAMES_SHADER_SHA256: &str =
    "0135e91818d1afa17c50a6e88160d68a0a5edda76561d0de2a76f6e5e53bc3d5";
const BUTTONSTATE_BLEND_FRAMES_SHADER_BYTES: u64 = 2_461;
const STATIC_BUTTON_SHADER_SHA256: &str =
    "fe3c84ec351378044aca0e09cdddbc241ddab29e15b65cd21bc76b1d4e91ce00";
const STATIC_BUTTON_SHADER_BYTES: u64 = 1_499;
const BUTTONSTATE_INCLUDE_SHA256: &str =
    "8a0e70eb222f1d71b1660fad3c19625859e95e72959995b3f26d478a8b95dd93";
const BUTTONSTATE_INCLUDE_BYTES: u64 = 243;
const SPRITE_ANIMATION_INCLUDE_SHA256: &str =
    "b32e1007b2e41df9ad3c41f49c6df32b7f7fce58bf0f86f55207505a1382238f";
const SPRITE_ANIMATION_INCLUDE_BYTES: u64 = 4_204;

pub(crate) fn authenticate_xui23_buttonstate_shader(
    resolver: &AssetResolver,
    sprite: &SpriteDefinition,
) -> Result<(), RuntimeGraphicsRejection> {
    let effect = crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref())
        .map_err(|_| RuntimeGraphicsRejection::UnsupportedEffectiveShader)?;
    if !effect.eq_ignore_ascii_case("gfx/FX/buttonstate.shader") {
        return Err(RuntimeGraphicsRejection::UnsupportedEffectiveShader);
    }
    for (path, expected_sha256, expected_bytes, include) in [
        (
            "gfx/FX/buttonstate.shader",
            BUTTONSTATE_SHADER_SHA256,
            BUTTONSTATE_SHADER_BYTES,
            false,
        ),
        (
            "gfx/FX/buttonstate.fxh",
            BUTTONSTATE_INCLUDE_SHA256,
            BUTTONSTATE_INCLUDE_BYTES,
            true,
        ),
        (
            "gfx/FX/sprite_animation.fxh",
            SPRITE_ANIMATION_INCLUDE_SHA256,
            SPRITE_ANIMATION_INCLUDE_BYTES,
            true,
        ),
    ] {
        let mismatch = || {
            if include {
                RuntimeGraphicsRejection::BuiltinShaderIncludeMismatch
            } else {
                RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch
            }
        };
        let resolved = resolver.resolve_file(path).map_err(|_| mismatch())?;
        let bytes = resolver.read_file(&resolved).map_err(|_| mismatch())?;
        if !resolved.rel_path.eq_ignore_ascii_case(path)
            || bytes.len() as u64 != expected_bytes
            || Sha256::hex(&bytes) != expected_sha256
        {
            return Err(mismatch());
        }
    }
    Ok(())
}

pub(crate) fn authenticate_xui23_progress_shader(
    resolver: &AssetResolver,
    sprite: &SpriteDefinition,
) -> Result<(), RuntimeGraphicsRejection> {
    let effect = crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref())
        .map_err(|_| RuntimeGraphicsRejection::UnsupportedEffectiveShader)?;
    if !effect.eq_ignore_ascii_case("gfx/FX/progress.shader") {
        return Err(RuntimeGraphicsRejection::UnsupportedEffectiveShader);
    }
    let resolved = resolver
        .resolve_file("gfx/FX/progress.shader")
        .map_err(|_| RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch)?;
    let bytes = resolver.read_file(&resolved)
        .map_err(|_| RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch)?;
    if !resolved
        .rel_path
        .eq_ignore_ascii_case("gfx/FX/progress.shader")
        || bytes.len() as u64 != PROGRESS_SHADER_BYTES
        || Sha256::hex(&bytes) != PROGRESS_SHADER_SHA256
    {
        return Err(RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch);
    }
    Ok(())
}
pub const TEXT_SHADER_SHA256: &str =
    "ae3b1e284881127d9f069d3af383e445bdd61ca40c00bf3295e74f2d87ab149e";
pub const TEXT_SHADER_BYTES: u64 = 1_682;
const PROGRESS_SHADER_SHA256: &str =
    "75cbb6f15ebdd503e2b4bc8ce03a17ad85c05f0fe127b86f3fb065546071f525";
const PROGRESS_SHADER_BYTES: u64 = 1_656;
const PROGRESS_START_END_SHADER_SHA256: &str =
    "b0bab8b288a0d5c664c5b43ed0db50fb3d65ace2d23e51aad1db3007da633d65";
const PROGRESS_START_END_SHADER_BYTES: u64 = 2_246;
const PROGRESS_REVERSE_SHADER_SHA256: &str =
    "4d0e66eef57ad8b95ee9aa94af1774e73177bbc47b9c74fc6962282e5da17bc4";
const PROGRESS_REVERSE_SHADER_BYTES: u64 = 2_258;
const PROGRESS_MIN_MAX_SHADER_SHA256: &str =
    "3ebc55f4cf2feec6232bedb30b7415f3e29075f02cfb1a967c0c286b6e61855d";
const PROGRESS_MIN_MAX_SHADER_BYTES: u64 = 1_896;
const PROGRESS_RADIAL_SHADER_SHA256: &str =
    "450982677dfbde87fcb775aa369401d7a7541eb317c953886be6e857e9519374";
const PROGRESS_RADIAL_SHADER_BYTES: u64 = 1_799;
const CIRCULAR_PROGRESS_SHADER_SHA256: &str =
    "f1e36411138bb55faeb41612ad6280da8131bb24b70b4e882090002b5600b0c7";
const CIRCULAR_PROGRESS_SHADER_BYTES: u64 = 1_585;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeGraphicsRequest {
    pub source_record_id: String,
    pub sprite_name: String,
    pub frame_count: usize,
    pub one_based_frame: u32,
    pub interaction_effect: ButtonVisualState,
    pub shading_path: ShadingPath,
    pub producer_resolved_rgba: Option<RgbaF32>,
    pub color: RgbaF32,
    pub time: f32,
    pub animation_time: f32,
    pub next_one_based_frame: Option<u32>,
    pub clip: ClipRect,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeGraphicsRejection {
    MissingProvenance,
    InvalidFrame,
    InvalidClip,
    InvalidOpacity,
    InvalidShaderConstants,
    MissingNextFrame,
    UnexpectedNextFrame,
    MissingProducerResolvedShading,
    UnindexedElement,
    SpriteIdentityMismatch,
    SpriteFrameCountMismatch,
    BuiltinShaderLawUnbound,
    ProducerResolvedPixelsUnbound,
    UnsupportedEffectiveShader,
    BuiltinShaderLawHashMismatch,
    BuiltinShaderIncludeMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdmittedRuntimeGraphics {
    frame: FrameSelection,
    next_frame: Option<FrameSelection>,
    shading_path: ShadingPath,
    shader_law: Option<AuthenticatedUiShaderLaw>,
    clip: ClipRect,
    opacity: f32,
    producer_resolved_rgba: Option<RgbaF32>,
    interaction_effect: ButtonVisualState,
    color: RgbaF32,
    time: f32,
    animation_time: f32,
}

impl AdmittedRuntimeGraphics {
    pub fn frame(self) -> FrameSelection {
        self.frame
    }

    pub fn shading_path(self) -> ShadingPath {
        self.shading_path
    }

    pub fn next_frame(self) -> Option<FrameSelection> {
        self.next_frame
    }

    pub fn clip(self) -> ClipRect {
        self.clip
    }

    pub fn opacity(self) -> f32 {
        self.opacity
    }

    pub fn producer_resolved_rgba(self) -> Option<RgbaF32> {
        self.producer_resolved_rgba
    }

    pub fn interaction_effect(self) -> ButtonVisualState {
        self.interaction_effect
    }

    pub fn color(self) -> RgbaF32 {
        self.color
    }

    pub fn time(self) -> f32 {
        self.time
    }

    pub fn animation_time(self) -> f32 {
        self.animation_time
    }
}

pub fn admit_runtime_graphics(
    request: &RuntimeGraphicsRequest,
) -> Result<AdmittedRuntimeGraphics, RuntimeGraphicsRejection> {
    if request.source_record_id.trim().is_empty() || request.sprite_name.trim().is_empty() {
        return Err(RuntimeGraphicsRejection::MissingProvenance);
    }
    let frame = select_one_based_frame(request.one_based_frame, request.frame_count)
        .ok_or(RuntimeGraphicsRejection::InvalidFrame)?;
    if !request.clip.is_nonempty() {
        return Err(RuntimeGraphicsRejection::InvalidClip);
    }
    if !request.opacity.is_finite() || !(0.0..=1.0).contains(&request.opacity) {
        return Err(RuntimeGraphicsRejection::InvalidOpacity);
    }
    if !request.color.finite() || !request.time.is_finite() || !request.animation_time.is_finite() {
        return Err(RuntimeGraphicsRejection::InvalidShaderConstants);
    }
    let next_frame = request
        .next_one_based_frame
        .map(|frame| {
            select_one_based_frame(frame, request.frame_count)
                .ok_or(RuntimeGraphicsRejection::InvalidFrame)
        })
        .transpose()?;
    if request.shading_path == ShadingPath::ButtonStateBlendFrames {
        if next_frame.is_none() || !(0.0..=1.0).contains(&request.animation_time) {
            return Err(RuntimeGraphicsRejection::MissingNextFrame);
        }
    } else if next_frame.is_some() {
        return Err(RuntimeGraphicsRejection::UnexpectedNextFrame);
    }
    if request.shading_path == ShadingPath::ProducerResolved
        && !request.producer_resolved_rgba.is_some_and(RgbaF32::finite)
    {
        return Err(RuntimeGraphicsRejection::MissingProducerResolvedShading);
    }
    Ok(AdmittedRuntimeGraphics {
        frame,
        next_frame,
        shading_path: request.shading_path,
        shader_law: None,
        clip: request.clip,
        opacity: request.opacity,
        producer_resolved_rgba: request.producer_resolved_rgba,
        interaction_effect: request.interaction_effect,
        color: request.color,
        time: request.time,
        animation_time: request.animation_time,
    })
}

fn admit_content_bound_runtime_graphics(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    element_id: &str,
    request: &RuntimeGraphicsRequest,
) -> Result<AdmittedRuntimeGraphics, RuntimeGraphicsRejection> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| RuntimeGraphicsRejection::UnindexedElement)?;
    let element = surface
        .elements
        .iter()
        .find(|element| element.element_id == element_id)
        .ok_or(RuntimeGraphicsRejection::UnindexedElement)?;
    let sprite = element
        .sprites
        .iter()
        .find(|sprite| {
            sprite
                .sprite_name
                .eq_ignore_ascii_case(&request.sprite_name)
        })
        .ok_or(RuntimeGraphicsRejection::SpriteIdentityMismatch)?;
    if usize::try_from(sprite.frame_count).ok() != Some(request.frame_count) {
        return Err(RuntimeGraphicsRejection::SpriteFrameCountMismatch);
    }
    let law = authenticate_builtin_shader_law(sprite)?;
    if request.shading_path != law.shading_path() {
        return Err(RuntimeGraphicsRejection::BuiltinShaderLawUnbound);
    }
    let mut admitted = admit_runtime_graphics(request)?;
    admitted.shader_law = Some(law);
    Ok(admitted)
}

pub(crate) fn admit_authenticated_widget_runtime_graphics(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    state: &AuthenticatedWidgetGraphicsState,
    sprite_name: &str,
    clip: ClipRect,
) -> Result<AdmittedRuntimeGraphics, RuntimeGraphicsRejection> {
    if !panel_lease.remains_current()
        || panel_lease.window_id() != panel.window_id()
        || panel_lease.runtime_instance_token() != state.runtime_instance_token()
    {
        return Err(RuntimeGraphicsRejection::UnindexedElement);
    }
    let identity = state.identity();
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| RuntimeGraphicsRejection::UnindexedElement)?;
    let element = surface
        .elements
        .iter()
        .find(|element| element.child_path == identity.child_path())
        .ok_or(RuntimeGraphicsRejection::UnindexedElement)?;
    if element.element_id != identity.element_id()
        || !element
            .authored_name
            .eq_ignore_ascii_case(identity.authored_name())
        || element.element_type != identity.element_type()
    {
        return Err(RuntimeGraphicsRejection::UnindexedElement);
    }
    let sprite = element
        .sprites
        .iter()
        .find(|sprite| sprite.sprite_name.eq_ignore_ascii_case(sprite_name))
        .ok_or(RuntimeGraphicsRejection::SpriteIdentityMismatch)?;
    let frame_count = usize::try_from(sprite.frame_count)
        .map_err(|_| RuntimeGraphicsRejection::SpriteFrameCountMismatch)?;
    let law = authenticate_builtin_shader_law(sprite)?;
    if law == AuthenticatedUiShaderLaw::ButtonStateBlendFrames {
        return Err(RuntimeGraphicsRejection::MissingNextFrame);
    }
    if state.observed_at_micros() < state.effect_entered_at_micros() {
        return Err(RuntimeGraphicsRejection::InvalidShaderConstants);
    }
    admit_content_bound_runtime_graphics(
        index,
        admission,
        panel,
        identity.element_id(),
        &RuntimeGraphicsRequest {
            source_record_id: state.runtime_instance_token().to_string(),
            sprite_name: sprite_name.to_string(),
            frame_count,
            one_based_frame: state.frame().one_based_frame,
            interaction_effect: state.interaction(),
            shading_path: law.shading_path(),
            producer_resolved_rgba: None,
            color: RgbaF32(sprite.authored_color_rgba_f32_bits.map(f32::from_bits)),
            time: state.observed_at_micros() as f32 / 1_000_000.0,
            animation_time: state.effect_entered_at_micros() as f32 / 1_000_000.0,
            next_one_based_frame: None,
            clip,
            opacity: 1.0,
        },
    )
}

pub(crate) fn admit_authenticated_semantic_runtime_graphics(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    content_panel: &StrictAuthoredPanelAdmission,
    runtime_owner_panel: &StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    child_path: &[u32],
    sprite_name: &str,
    one_based_frame: u32,
    next_one_based_frame: Option<u32>,
    animation_time: f32,
    interaction: ButtonVisualState,
    clip: ClipRect,
) -> Result<AdmittedRuntimeGraphics, RuntimeGraphicsRejection> {
    if !frame_authority.remains_staged_for(manager)
        || !panel_lease.remains_current()
        || panel_lease.window_id() != runtime_owner_panel.window_id()
        || panel_lease.runtime_instance_token().trim().is_empty()
    {
        return Err(RuntimeGraphicsRejection::UnindexedElement);
    }
    let surface = index
        .validate_panel_token(admission, content_panel)
        .map_err(|_| RuntimeGraphicsRejection::UnindexedElement)?;
    index
        .validate_panel_token(admission, runtime_owner_panel)
        .map_err(|_| RuntimeGraphicsRejection::UnindexedElement)?;
    let element = surface
        .elements
        .iter()
        .find(|element| element.child_path == child_path)
        .ok_or(RuntimeGraphicsRejection::UnindexedElement)?;
    let sprite = element
        .sprites
        .iter()
        .find(|sprite| sprite.sprite_name.eq_ignore_ascii_case(sprite_name))
        .ok_or(RuntimeGraphicsRejection::SpriteIdentityMismatch)?;
    let frame_count = usize::try_from(sprite.frame_count)
        .map_err(|_| RuntimeGraphicsRejection::SpriteFrameCountMismatch)?;
    let law = authenticate_builtin_shader_law(sprite)?;
    if (law == AuthenticatedUiShaderLaw::ButtonStateBlendFrames) != next_one_based_frame.is_some() {
        return Err(if law == AuthenticatedUiShaderLaw::ButtonStateBlendFrames {
            RuntimeGraphicsRejection::MissingNextFrame
        } else {
            RuntimeGraphicsRejection::UnexpectedNextFrame
        });
    }
    admit_content_bound_runtime_graphics(
        index,
        admission,
        content_panel,
        &element.element_id,
        &RuntimeGraphicsRequest {
            source_record_id: panel_lease.runtime_instance_token().to_string(),
            sprite_name: sprite_name.to_string(),
            frame_count,
            one_based_frame,
            interaction_effect: interaction,
            shading_path: law.shading_path(),
            producer_resolved_rgba: None,
            color: RgbaF32(sprite.authored_color_rgba_f32_bits.map(f32::from_bits)),
            time: 0.0,
            animation_time,
            next_one_based_frame,
            clip,
            opacity: 1.0,
        },
    )
}

/// authored colour, frame, clip, and current runtime ownership.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit_authenticated_dynamic_sprite_runtime_graphics(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    runtime_owner_panel: &StrictAuthoredPanelAdmission,
    panel_lease: &AuthenticatedReplayPanelLease<'_>,
    sprite_name: &str,
    one_based_frame: u32,
    clip: ClipRect,
) -> Result<AdmittedRuntimeGraphics, RuntimeGraphicsRejection> {
    if !frame_authority.remains_staged_for(manager)
        || !panel_lease.remains_current()
        || panel_lease.window_id() != runtime_owner_panel.window_id()
        || panel_lease.runtime_instance_token().trim().is_empty()
    {
        return Err(RuntimeGraphicsRejection::UnindexedElement);
    }
    index
        .validate_panel_token(admission, runtime_owner_panel)
        .map_err(|_| RuntimeGraphicsRejection::UnindexedElement)?;
    let sprite = index
        .admit_dynamic_sprite(admission, sprite_name)
        .map_err(|_| RuntimeGraphicsRejection::SpriteIdentityMismatch)?;
    let frame_count = usize::try_from(sprite.frame_count)
        .map_err(|_| RuntimeGraphicsRejection::SpriteFrameCountMismatch)?;
    let law = authenticate_builtin_shader_law(sprite)?;
    if law == AuthenticatedUiShaderLaw::ButtonStateBlendFrames {
        return Err(RuntimeGraphicsRejection::MissingNextFrame);
    }
    let mut admitted = admit_runtime_graphics(&RuntimeGraphicsRequest {
        source_record_id: panel_lease.runtime_instance_token().to_string(),
        sprite_name: sprite_name.to_string(),
        frame_count,
        one_based_frame,
        interaction_effect: ButtonVisualState::Up,
        shading_path: law.shading_path(),
        producer_resolved_rgba: None,
        color: RgbaF32(sprite.authored_color_rgba_f32_bits.map(f32::from_bits)),
        time: 0.0,
        animation_time: 0.0,
        next_one_based_frame: None,
        clip,
        opacity: 1.0,
    })?;
    admitted.shader_law = Some(law);
    Ok(admitted)
}

fn authenticate_builtin_shader_law(
    sprite: &crate::authored_role_index::AuthoredSpriteRole,
) -> Result<AuthenticatedUiShaderLaw, RuntimeGraphicsRejection> {
    let (law, shader_sha256, shader_bytes, includes): (
        AuthenticatedUiShaderLaw,
        &str,
        u64,
        &[(&str, &str, u64)],
    ) = if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/buttonstate.shader")
    {
        (
            AuthenticatedUiShaderLaw::ButtonState,
            BUTTONSTATE_SHADER_SHA256,
            BUTTONSTATE_SHADER_BYTES,
            &[
                (
                    "gfx/FX/buttonstate.fxh",
                    BUTTONSTATE_INCLUDE_SHA256,
                    BUTTONSTATE_INCLUDE_BYTES,
                ),
                (
                    "gfx/FX/sprite_animation.fxh",
                    SPRITE_ANIMATION_INCLUDE_SHA256,
                    SPRITE_ANIMATION_INCLUDE_BYTES,
                ),
            ],
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/buttonstate_nodowneffect.shader")
    {
        (
            AuthenticatedUiShaderLaw::ButtonStateNoDownEffect,
            BUTTONSTATE_NO_DOWN_EFFECT_SHADER_SHA256,
            BUTTONSTATE_NO_DOWN_EFFECT_SHADER_BYTES,
            &[],
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/buttonstate_nodownordisableeffect.shader")
    {
        (
            AuthenticatedUiShaderLaw::ButtonStateNoDownOrDisableEffect,
            BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_SHA256,
            BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_BYTES,
            &[],
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/buttonstate_onlydisable.shader")
    {
        (
            AuthenticatedUiShaderLaw::ButtonStateOnlyDisable,
            BUTTONSTATE_ONLY_DISABLE_SHADER_SHA256,
            BUTTONSTATE_ONLY_DISABLE_SHADER_BYTES,
            &[
                (
                    "gfx/FX/buttonstate.fxh",
                    BUTTONSTATE_INCLUDE_SHA256,
                    BUTTONSTATE_INCLUDE_BYTES,
                ),
                (
                    "gfx/FX/sprite_animation.fxh",
                    SPRITE_ANIMATION_INCLUDE_SHA256,
                    SPRITE_ANIMATION_INCLUDE_BYTES,
                ),
            ],
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/buttonstate_blendframes.shader")
    {
        (
            AuthenticatedUiShaderLaw::ButtonStateBlendFrames,
            BUTTONSTATE_BLEND_FRAMES_SHADER_SHA256,
            BUTTONSTATE_BLEND_FRAMES_SHADER_BYTES,
            &[(
                "gfx/FX/buttonstate.fxh",
                BUTTONSTATE_INCLUDE_SHA256,
                BUTTONSTATE_INCLUDE_BYTES,
            )],
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/static_button.shader")
    {
        (
            AuthenticatedUiShaderLaw::StaticButton,
            STATIC_BUTTON_SHADER_SHA256,
            STATIC_BUTTON_SHADER_BYTES,
            &[],
        )
    } else {
        return Err(RuntimeGraphicsRejection::UnsupportedEffectiveShader);
    };
    if !content_matches(
        &sprite.effect_content,
        &sprite.effective_shader_file,
        shader_sha256,
        shader_bytes,
    ) {
        return Err(RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch);
    }
    if sprite.effect_include_content.len() != includes.len()
        || sprite
            .effect_include_content
            .iter()
            .zip(includes)
            .any(|(content, (path, sha256, bytes))| !content_matches(content, path, sha256, *bytes))
    {
        return Err(RuntimeGraphicsRejection::BuiltinShaderIncludeMismatch);
    }
    Ok(law)
}

pub(crate) fn authenticate_progress_shader_law(
    sprite: &crate::authored_role_index::AuthoredSpriteRole,
    fill: ProgressFill,
) -> Result<AuthenticatedProgressShaderLaw, RuntimeGraphicsRejection> {
    if !fill.valid() {
        return Err(RuntimeGraphicsRejection::InvalidShaderConstants);
    }
    let (law, effect, sha256, bytes) = if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/progress.shader")
    {
        (
            AuthenticatedProgressShaderLaw::Linear,
            ProgressEffect::Linear,
            PROGRESS_SHADER_SHA256,
            PROGRESS_SHADER_BYTES,
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/progress_startend.shader")
    {
        (
            AuthenticatedProgressShaderLaw::StartEnd,
            ProgressEffect::StartEnd,
            PROGRESS_START_END_SHADER_SHA256,
            PROGRESS_START_END_SHADER_BYTES,
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/progress_reverse.shader")
    {
        (
            AuthenticatedProgressShaderLaw::Reverse,
            ProgressEffect::Reverse,
            PROGRESS_REVERSE_SHADER_SHA256,
            PROGRESS_REVERSE_SHADER_BYTES,
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/progress_minmax.shader")
    {
        let effect = match fill.effect {
            ProgressEffect::MinMaxColor => ProgressEffect::MinMaxColor,
            ProgressEffect::MinMaxTexture => ProgressEffect::MinMaxTexture,
            _ => return Err(RuntimeGraphicsRejection::UnsupportedEffectiveShader),
        };
        (
            AuthenticatedProgressShaderLaw::MinMax,
            effect,
            PROGRESS_MIN_MAX_SHADER_SHA256,
            PROGRESS_MIN_MAX_SHADER_BYTES,
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/progress_radial.shader")
    {
        (
            AuthenticatedProgressShaderLaw::Radial,
            ProgressEffect::Radial,
            PROGRESS_RADIAL_SHADER_SHA256,
            PROGRESS_RADIAL_SHADER_BYTES,
        )
    } else if sprite
        .effective_shader_file
        .eq_ignore_ascii_case("gfx/FX/circularprogressbar.shader")
    {
        (
            AuthenticatedProgressShaderLaw::Circular,
            ProgressEffect::Circular,
            CIRCULAR_PROGRESS_SHADER_SHA256,
            CIRCULAR_PROGRESS_SHADER_BYTES,
        )
    } else {
        return Err(RuntimeGraphicsRejection::UnsupportedEffectiveShader);
    };
    if fill.effect != effect
        || !content_matches(
            &sprite.effect_content,
            &sprite.effective_shader_file,
            sha256,
            bytes,
        )
    {
        return Err(RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch);
    }
    if !sprite.effect_include_content.is_empty() {
        return Err(RuntimeGraphicsRejection::BuiltinShaderIncludeMismatch);
    }
    Ok(law)
}

fn content_matches(
    content: &portable_content_manifest::UsedContentKey,
    path: &str,
    sha256: &str,
    bytes: u64,
) -> bool {
    content
        .normalized_game_relative_path
        .eq_ignore_ascii_case(path)
        && content.payload_sha256 == sha256
        && content.payload_bytes == bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedSpriteGeometry {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub border: Option<(usize, usize)>,
}

pub(crate) fn draw_authenticated_sprite_pixels(
    destination: &mut RgbaImage,
    draw: crate::compose::AuthenticatedSpriteDraw<'_>,
) -> Result<usize, RuntimeGraphicsRejection> {
    let (atlas, frame_count, geometry, admitted) = draw.into_parts();
    draw_authenticated_sprite_pixels_impl(destination, atlas, frame_count, geometry, admitted)
}

fn draw_authenticated_sprite_pixels_impl(
    destination: &mut RgbaImage,
    atlas: &RgbaImage,
    frame_count: usize,
    geometry: AuthenticatedSpriteGeometry,
    admitted: AdmittedRuntimeGraphics,
) -> Result<usize, RuntimeGraphicsRejection> {
    let law = admitted
        .shader_law
        .ok_or(RuntimeGraphicsRejection::BuiltinShaderLawUnbound)?;
    if frame_count == 0
        || atlas.width == 0
        || atlas.height == 0
        || atlas.width % frame_count != 0
        || admitted.frame.zero_based_index >= frame_count
        || admitted
            .next_frame
            .is_some_and(|frame| frame.zero_based_index >= frame_count)
    {
        return Err(RuntimeGraphicsRejection::InvalidFrame);
    }
    let frame_width = atlas.width / frame_count;
    if frame_width == 0 {
        return Err(RuntimeGraphicsRejection::InvalidFrame);
    }
    if geometry.width == 0 || geometry.height == 0 {
        return Ok(0);
    }

    let mut drawn = 0usize;
    let x_range = clipped_axis_range(
        geometry.x,
        geometry.width,
        destination.width,
        admitted.clip.left,
        admitted.clip.right,
    );
    let y_range = clipped_axis_range(
        geometry.y,
        geometry.height,
        destination.height,
        admitted.clip.top,
        admitted.clip.bottom,
    );
    for local_y in y_range {
        let y = i64::from(geometry.y) + local_y as i64;
        for local_x in x_range.clone() {
            let x = i64::from(geometry.x) + local_x as i64;
            let source_x = mapped_source_coordinate(
                local_x,
                geometry.width,
                frame_width,
                geometry.border.map(|value| value.0),
            ) + (admitted.frame.zero_based_index * frame_width) as f32;
            let source_y = mapped_source_coordinate(
                local_y,
                geometry.height,
                atlas.height,
                geometry.border.map(|value| value.1),
            );
            let base = match law {
                AuthenticatedUiShaderLaw::StaticButton => sample_point_addressed(
                    atlas,
                    source_x,
                    source_y,
                    SampleAddressMode::Wrap,
                    SampleAddressMode::Wrap,
                ),
                AuthenticatedUiShaderLaw::ButtonStateNoDownEffect
                | AuthenticatedUiShaderLaw::ButtonStateNoDownOrDisableEffect => {
                    sample_point_addressed(
                        atlas,
                        source_x,
                        source_y,
                        SampleAddressMode::Clamp,
                        SampleAddressMode::Clamp,
                    )
                }
                AuthenticatedUiShaderLaw::ButtonState => sample_linear(atlas, source_x, source_y),
                AuthenticatedUiShaderLaw::ButtonStateOnlyDisable => sample_linear_addressed(
                    atlas,
                    source_x,
                    source_y,
                    SampleAddressMode::Wrap,
                    SampleAddressMode::Wrap,
                ),
                AuthenticatedUiShaderLaw::ButtonStateBlendFrames => sample_linear_addressed(
                    atlas,
                    source_x,
                    source_y,
                    SampleAddressMode::Wrap,
                    SampleAddressMode::Clamp,
                ),
            };
            let shaded = match law {
                AuthenticatedUiShaderLaw::StaticButton => shade_static_button(base, admitted.color),
                AuthenticatedUiShaderLaw::ButtonState => shade_buttonstate(
                    base,
                    admitted.color,
                    admitted.interaction_effect,
                    admitted.time,
                    admitted.animation_time,
                ),
                AuthenticatedUiShaderLaw::ButtonStateNoDownEffect => {
                    shade_buttonstate_no_down_effect(
                        base,
                        admitted.color,
                        admitted.interaction_effect,
                    )
                }
                AuthenticatedUiShaderLaw::ButtonStateNoDownOrDisableEffect => {
                    shade_buttonstate_no_down_or_disable_effect(
                        base,
                        admitted.color,
                        admitted.interaction_effect,
                    )
                }
                AuthenticatedUiShaderLaw::ButtonStateOnlyDisable => shade_buttonstate_only_disable(
                    base,
                    admitted.color,
                    admitted.interaction_effect,
                ),
                AuthenticatedUiShaderLaw::ButtonStateBlendFrames => {
                    let next = admitted
                        .next_frame
                        .ok_or(RuntimeGraphicsRejection::MissingNextFrame)?;
                    let next_x = mapped_source_coordinate(
                        local_x,
                        geometry.width,
                        frame_width,
                        geometry.border.map(|value| value.0),
                    ) + (next.zero_based_index * frame_width) as f32;
                    let next_y = mapped_source_coordinate(
                        local_y,
                        geometry.height,
                        atlas.height,
                        geometry.border.map(|value| value.1),
                    );
                    shade_buttonstate_blendframes(
                        base,
                        sample_linear_addressed(
                            atlas,
                            next_x,
                            next_y,
                            SampleAddressMode::Wrap,
                            SampleAddressMode::Clamp,
                        ),
                        admitted.animation_time,
                        admitted.color,
                        admitted.interaction_effect,
                    )
                }
            };
            let source = apply_opacity(shaded, admitted.opacity);
            let offset = (y as usize * destination.width + x as usize) * 4;
            let destination_pixel = rgba_u8_to_f32(
                destination.pixels[offset..offset + 4]
                    .try_into()
                    .expect("RGBA destination slice"),
            );
            let out = blend_src_alpha_inv_src_alpha(destination_pixel, source);
            destination.pixels[offset..offset + 4].copy_from_slice(&rgba_f32_to_u8(out));
            drawn += 1;
        }
    }
    Ok(drawn)
}

pub(crate) struct AuthenticatedProgressDraw<'a> {
    foreground: &'a RgbaImage,
    background: &'a RgbaImage,
    geometry: AuthenticatedSpriteGeometry,
    clip: ClipRect,
    fill: ProgressFill,
    law: AuthenticatedProgressShaderLaw,
}

pub(crate) fn admit_authenticated_progress_draw<'a>(
    sprite: &crate::authored_role_index::AuthoredSpriteRole,
    foreground: &'a RgbaImage,
    background: &'a RgbaImage,
    geometry: AuthenticatedSpriteGeometry,
    clip: ClipRect,
    fill: ProgressFill,
) -> Result<AuthenticatedProgressDraw<'a>, RuntimeGraphicsRejection> {
    let law = authenticate_progress_shader_law(sprite, fill)?;
    if sprite.frame_count != 1
        || sprite.texture_slots.len() != 2
        || geometry.border.is_some()
        || !clip.is_nonempty()
        || foreground.width == 0
        || foreground.height == 0
        || background.width == 0
        || background.height == 0
    {
        return Err(RuntimeGraphicsRejection::SpriteFrameCountMismatch);
    }
    if fill.effect == ProgressEffect::MinMaxColor {
        return Err(RuntimeGraphicsRejection::MissingProducerResolvedShading);
    }
    Ok(AuthenticatedProgressDraw {
        foreground,
        background,
        geometry,
        clip,
        fill,
        law,
    })
}

pub(crate) fn draw_authenticated_progress_pixels(
    destination: &mut RgbaImage,
    draw: AuthenticatedProgressDraw<'_>,
) -> Result<usize, RuntimeGraphicsRejection> {
    if draw.geometry.width == 0 || draw.geometry.height == 0 {
        return Ok(0);
    }
    let x_range = clipped_axis_range(
        draw.geometry.x,
        draw.geometry.width,
        destination.width,
        draw.clip.left,
        draw.clip.right,
    );
    let y_range = clipped_axis_range(
        draw.geometry.y,
        draw.geometry.height,
        destination.height,
        draw.clip.top,
        draw.clip.bottom,
    );
    let mut drawn = 0_usize;
    for local_y in y_range {
        let y = i64::from(draw.geometry.y) + local_y as i64;
        let input_v = (local_y as f32 + 0.5) / draw.geometry.height as f32;
        for local_x in x_range.clone() {
            let x = i64::from(draw.geometry.x) + local_x as i64;
            let input_u = (local_x as f32 + 0.5) / draw.geometry.width as f32;
            let source = shade_progress_texture_pixel(
                draw.foreground,
                draw.background,
                draw.law,
                draw.fill,
                input_u,
                input_v,
            )?;
            let offset = (y as usize * destination.width + x as usize) * 4;
            let destination_pixel = rgba_u8_to_f32(
                destination.pixels[offset..offset + 4]
                    .try_into()
                    .expect("RGBA destination slice"),
            );
            let out = blend_src_alpha_inv_src_alpha(destination_pixel, source);
            destination.pixels[offset..offset + 4].copy_from_slice(&rgba_f32_to_u8(out));
            drawn += 1;
        }
    }
    Ok(drawn)
}

fn shade_progress_texture_pixel(
    foreground: &RgbaImage,
    background: &RgbaImage,
    law: AuthenticatedProgressShaderLaw,
    fill: ProgressFill,
    input_u: f32,
    input_v: f32,
) -> Result<RgbaF32, RuntimeGraphicsRejection> {
    let flipped_v = -input_v;
    match law {
        AuthenticatedProgressShaderLaw::Linear => {
            let texture = if input_u <= fill.current_state {
                foreground
            } else {
                background
            };
            Ok(sample_point_normalized_wrap(texture, input_u, flipped_v))
        }
        AuthenticatedProgressShaderLaw::StartEnd | AuthenticatedProgressShaderLaw::Reverse => {
            let foreground_branch = match law {
                AuthenticatedProgressShaderLaw::StartEnd => input_u <= fill.current_state,
                AuthenticatedProgressShaderLaw::Reverse => input_u >= 1.0 - fill.current_state,
                _ => unreachable!(),
            };
            if !foreground_branch {
                return Ok(sample_point_normalized_wrap(background, input_u, flipped_v));
            }
            let start_v = flipped_v / 3.0 + 1.0 / 3.0;
            let middle_v = start_v + 1.0 / 3.0;
            let stop_v = middle_v + 1.0 / 3.0;
            let stop_u = input_u + 1.0 - fill.current_state;
            let start = sample_point_normalized_wrap(foreground, input_u, start_v);
            let middle = sample_point_normalized_wrap(foreground, input_u, middle_v);
            let stop = sample_point_normalized_wrap(foreground, stop_u, stop_v);
            let mut color = lerp_rgba(middle, start, start.0[3]);
            color = lerp_rgba(color, stop, stop.0[3]);
            color.0[3] = middle.0[3];
            Ok(color)
        }
        AuthenticatedProgressShaderLaw::MinMax => {
            let use_foreground = (input_u < 0.5 && input_u >= fill.current_state)
                || (input_u >= 0.5 && input_u <= fill.current_state);
            let texture = if use_foreground {
                foreground
            } else {
                background
            };
            Ok(sample_point_normalized_wrap(texture, input_u, flipped_v))
        }
        AuthenticatedProgressShaderLaw::Radial => {
            let dx = 0.5 - input_u;
            let dy = 0.5 - input_v;
            let angle = dx.atan2(dy) + std::f32::consts::PI;
            let factor = saturate((angle - fill.current_state * std::f32::consts::PI * 2.0) * 50.0);
            let first = sample_point_normalized_wrap(foreground, input_u, input_v);
            let second = sample_point_normalized_wrap(background, input_u, input_v);
            Ok(lerp_rgba(first, second, factor))
        }
        AuthenticatedProgressShaderLaw::Circular => {
            let factor = fill
                .circular_mix
                .ok_or(RuntimeGraphicsRejection::InvalidShaderConstants)?;
            let u = input_u * 0.5 - 0.5;
            let v = 0.5 - input_v * 0.5;
            let first = sample_linear_normalized_wrap(foreground, u, v);
            let second = sample_linear_normalized_wrap(background, u, v);
            Ok(lerp_rgba(first, second, factor))
        }
    }
}

fn sample_point_normalized_wrap(image: &RgbaImage, u: f32, v: f32) -> RgbaF32 {
    sample_point_addressed(
        image,
        u * image.width as f32 - 0.5,
        v * image.height as f32 - 0.5,
        SampleAddressMode::Wrap,
        SampleAddressMode::Wrap,
    )
}

fn sample_linear_normalized_wrap(image: &RgbaImage, u: f32, v: f32) -> RgbaF32 {
    sample_linear_addressed(
        image,
        u * image.width as f32 - 0.5,
        v * image.height as f32 - 0.5,
        SampleAddressMode::Wrap,
        SampleAddressMode::Wrap,
    )
}

fn clipped_axis_range(
    origin: i32,
    target_extent: usize,
    destination_extent: usize,
    clip_start: i32,
    clip_end: i32,
) -> std::ops::Range<usize> {
    let origin = i64::from(origin);
    let start = 0i64.max(-origin).max(i64::from(clip_start) - origin);
    let end = (target_extent.min(i64::MAX as usize) as i64)
        .min(destination_extent.min(i64::MAX as usize) as i64 - origin)
        .min(i64::from(clip_end) - origin);
    if end <= start {
        0..0
    } else {
        start as usize..end as usize
    }
}

fn mapped_source_coordinate(
    local: usize,
    target_extent: usize,
    source_extent: usize,
    border: Option<usize>,
) -> f32 {
    let border = border
        .unwrap_or(0)
        .min(source_extent / 2)
        .min(target_extent / 2);
    let target_middle = target_extent.saturating_sub(border * 2);
    let source_middle = source_extent.saturating_sub(border * 2);
    let (target_start, target_len, source_start, source_len) = if local < border {
        (0, border, 0, border)
    } else if local >= target_extent.saturating_sub(border) {
        (
            target_extent.saturating_sub(border),
            border,
            source_extent.saturating_sub(border),
            border,
        )
    } else {
        (border, target_middle, border, source_middle)
    };
    if target_len == 0 || source_len == 0 {
        return 0.0;
    }
    source_start as f32
        + ((local - target_start) as f32 + 0.5) * source_len as f32 / target_len as f32
        - 0.5
}

fn sample_point(image: &RgbaImage, x: f32, y: f32) -> RgbaF32 {
    sample_point_addressed(
        image,
        x,
        y,
        SampleAddressMode::Clamp,
        SampleAddressMode::Clamp,
    )
}

fn sample_point_addressed(
    image: &RgbaImage,
    x: f32,
    y: f32,
    address_u: SampleAddressMode,
    address_v: SampleAddressMode,
) -> RgbaF32 {
    let x = point_sample_axis(x, image.width, address_u);
    let y = point_sample_axis(y, image.height, address_v);
    image_pixel(image, x, y)
}

fn point_sample_axis(coordinate: f32, extent: usize, address: SampleAddressMode) -> usize {
    let nearest = (coordinate + 0.5).floor() as i64;
    match address {
        SampleAddressMode::Clamp => nearest.clamp(0, extent.saturating_sub(1) as i64) as usize,
        SampleAddressMode::Wrap => nearest.rem_euclid(extent as i64) as usize,
    }
}

fn sample_linear(image: &RgbaImage, x: f32, y: f32) -> RgbaF32 {
    sample_linear_addressed(
        image,
        x,
        y,
        SampleAddressMode::Clamp,
        SampleAddressMode::Clamp,
    )
}

#[derive(Clone, Copy)]
enum SampleAddressMode {
    Clamp,
    Wrap,
}

fn sample_linear_addressed(
    image: &RgbaImage,
    x: f32,
    y: f32,
    address_u: SampleAddressMode,
    address_v: SampleAddressMode,
) -> RgbaF32 {
    let (x0, x1, fx) = linear_sample_axis(x, image.width, address_u);
    let (y0, y1, fy) = linear_sample_axis(y, image.height, address_v);
    let top = lerp_rgba(image_pixel(image, x0, y0), image_pixel(image, x1, y0), fx);
    let bottom = lerp_rgba(image_pixel(image, x0, y1), image_pixel(image, x1, y1), fx);
    lerp_rgba(top, bottom, fy)
}

fn linear_sample_axis(
    coordinate: f32,
    extent: usize,
    address: SampleAddressMode,
) -> (usize, usize, f32) {
    match address {
        SampleAddressMode::Clamp => {
            let coordinate = coordinate.clamp(0.0, (extent - 1) as f32);
            let first = coordinate.floor() as usize;
            let second = (first + 1).min(extent - 1);
            (first, second, coordinate - first as f32)
        }
        SampleAddressMode::Wrap => {
            let first_unwrapped = coordinate.floor() as i64;
            let extent = i64::try_from(extent).expect("decoded texture extent fits i64");
            let first = first_unwrapped.rem_euclid(extent) as usize;
            let second = (first_unwrapped + 1).rem_euclid(extent) as usize;
            (first, second, coordinate - first_unwrapped as f32)
        }
    }
}

fn image_pixel(image: &RgbaImage, x: usize, y: usize) -> RgbaF32 {
    let offset = (y * image.width + x) * 4;
    rgba_u8_to_f32(
        image.pixels[offset..offset + 4]
            .try_into()
            .expect("RGBA source slice"),
    )
}

fn rgba_u8_to_f32(pixel: [u8; 4]) -> RgbaF32 {
    RgbaF32(pixel.map(|channel| f32::from(channel) / 255.0))
}

fn rgba_f32_to_u8(pixel: RgbaF32) -> [u8; 4] {
    pixel
        .0
        .map(|channel| (saturate(channel) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{ContentLayerKind, UsedContentKey};

    fn content(path: &str, sha256: &str, bytes: u64) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: path.into(),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: sha256.into(),
            payload_bytes: bytes,
        }
    }

    fn buttonstate_role() -> crate::authored_role_index::AuthoredSpriteRole {
        crate::authored_role_index::AuthoredSpriteRole {
            sprite_name: "GFX_test".into(),
            sprite_kind: "spriteType".into(),
            frame_count: 1,
            authored_effect_file: None,
            effective_shader_file: "gfx/FX/buttonstate.shader".into(),
            definition_content: content("interface/test.gfx", &"c".repeat(64), 1),
            texture_slots: vec![crate::authored_role_index::AuthoredSpriteTextureRole {
                authored_slot_key: "texturefile".into(),
                authored_path: "gfx/interface/test.dds".into(),
                content: content("gfx/interface/test.dds", &"d".repeat(64), 1),
            }],
            effect_content: content(
                "gfx/FX/buttonstate.shader",
                BUTTONSTATE_SHADER_SHA256,
                BUTTONSTATE_SHADER_BYTES,
            ),
            effect_include_content: vec![
                content(
                    "gfx/FX/buttonstate.fxh",
                    BUTTONSTATE_INCLUDE_SHA256,
                    BUTTONSTATE_INCLUDE_BYTES,
                ),
                content(
                    "gfx/FX/sprite_animation.fxh",
                    SPRITE_ANIMATION_INCLUDE_SHA256,
                    SPRITE_ANIMATION_INCLUDE_BYTES,
                ),
            ],
            authored_color_rgba_f32_bits: [1.0f32.to_bits(); 4],
            authored_color_was_explicit: true,
        }
    }

    fn buttonstate_variant_role(
        path: &str,
        sha256: &str,
        bytes: u64,
        includes: Vec<UsedContentKey>,
    ) -> crate::authored_role_index::AuthoredSpriteRole {
        let mut role = buttonstate_role();
        role.authored_effect_file = Some(path.replace(".shader", ".lua"));
        role.effective_shader_file = path.into();
        role.effect_content = content(path, sha256, bytes);
        role.effect_include_content = includes;
        role
    }

    #[test]
    fn builtin_law_is_hash_and_include_order_bound() {
        let role = buttonstate_role();
        assert_eq!(
            authenticate_builtin_shader_law(&role),
            Ok(AuthenticatedUiShaderLaw::ButtonState)
        );

        let mut modified = role.clone();
        modified.effect_content.payload_sha256 = "0".repeat(64);
        assert_eq!(
            authenticate_builtin_shader_law(&modified),
            Err(RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch)
        );

        let mut reordered = role;
        reordered.effect_include_content.swap(0, 1);
        assert_eq!(
            authenticate_builtin_shader_law(&reordered),
            Err(RuntimeGraphicsRejection::BuiltinShaderIncludeMismatch)
        );
    }

    #[test]
    fn remaining_vanilla_buttonstate_laws_are_exact_content_bound() {
        let no_down = buttonstate_variant_role(
            "gfx/FX/buttonstate_nodowneffect.shader",
            BUTTONSTATE_NO_DOWN_EFFECT_SHADER_SHA256,
            BUTTONSTATE_NO_DOWN_EFFECT_SHADER_BYTES,
            Vec::new(),
        );
        assert_eq!(
            authenticate_builtin_shader_law(&no_down),
            Ok(AuthenticatedUiShaderLaw::ButtonStateNoDownEffect)
        );

        let no_down_or_disable = buttonstate_variant_role(
            "gfx/FX/buttonstate_nodownordisableeffect.shader",
            BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_SHA256,
            BUTTONSTATE_NO_DOWN_OR_DISABLE_EFFECT_SHADER_BYTES,
            Vec::new(),
        );
        assert_eq!(
            authenticate_builtin_shader_law(&no_down_or_disable),
            Ok(AuthenticatedUiShaderLaw::ButtonStateNoDownOrDisableEffect)
        );

        let only_disable = buttonstate_variant_role(
            "gfx/FX/buttonstate_onlydisable.shader",
            BUTTONSTATE_ONLY_DISABLE_SHADER_SHA256,
            BUTTONSTATE_ONLY_DISABLE_SHADER_BYTES,
            vec![
                content(
                    "gfx/FX/buttonstate.fxh",
                    BUTTONSTATE_INCLUDE_SHA256,
                    BUTTONSTATE_INCLUDE_BYTES,
                ),
                content(
                    "gfx/FX/sprite_animation.fxh",
                    SPRITE_ANIMATION_INCLUDE_SHA256,
                    SPRITE_ANIMATION_INCLUDE_BYTES,
                ),
            ],
        );
        assert_eq!(
            authenticate_builtin_shader_law(&only_disable),
            Ok(AuthenticatedUiShaderLaw::ButtonStateOnlyDisable)
        );

        let mut changed = no_down;
        changed.effect_content.payload_bytes += 1;
        assert_eq!(
            authenticate_builtin_shader_law(&changed),
            Err(RuntimeGraphicsRejection::BuiltinShaderLawHashMismatch)
        );
    }

    #[test]
    fn remaining_vanilla_buttonstate_pixel_variants_are_not_coerced_together() {
        let base = RgbaF32([0.4, 0.2, 0.8, 0.5]);
        let color = RgbaF32([0.5, 1.0, 0.25, 0.75]);

        assert_eq!(
            shade_buttonstate_no_down_effect(base, color, ButtonVisualState::Down),
            modulate_rgba(base, color)
        );
        assert_eq!(
            shade_buttonstate_only_disable(base, color, ButtonVisualState::Over),
            modulate_rgba(base, color)
        );
        assert_eq!(
            shade_buttonstate_no_down_or_disable_effect(base, color, ButtonVisualState::Disable),
            modulate_rgba(base, color)
        );
        assert_ne!(
            shade_buttonstate_no_down_effect(base, color, ButtonVisualState::Disable),
            modulate_rgba(base, color)
        );
        assert_ne!(
            shade_buttonstate_no_down_or_disable_effect(base, color, ButtonVisualState::Over),
            modulate_rgba(base, color)
        );
    }

    fn admitted(state: ButtonVisualState, clip: ClipRect) -> AdmittedRuntimeGraphics {
        AdmittedRuntimeGraphics {
            frame: FrameSelection {
                one_based_frame: 1,
                zero_based_index: 0,
            },
            next_frame: None,
            shading_path: ShadingPath::ButtonState,
            shader_law: Some(AuthenticatedUiShaderLaw::ButtonState),
            clip,
            opacity: 0.5,
            producer_resolved_rgba: None,
            interaction_effect: state,
            color: RgbaF32([1.0; 4]),
            time: 1.0,
            animation_time: 0.0,
        }
    }

    #[test]
    fn exact_sampler_address_modes_diverge_at_texture_edges() {
        assert_eq!(
            linear_sample_axis(-0.25, 2, SampleAddressMode::Clamp),
            (0, 1, 0.0)
        );
        assert_eq!(
            linear_sample_axis(-0.25, 2, SampleAddressMode::Wrap),
            (1, 0, 0.75)
        );
        assert_eq!(
            linear_sample_axis(1.25, 2, SampleAddressMode::Clamp),
            (1, 1, 0.0)
        );
        assert_eq!(
            linear_sample_axis(1.25, 2, SampleAddressMode::Wrap),
            (1, 0, 0.25)
        );
    }

    #[test]
    fn authenticated_multiframe_sampling_keeps_internal_atlas_seams() {
        let atlas = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let geometry = AuthenticatedSpriteGeometry {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            border: None,
        };
        let clip = ClipRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        };

        let mut first_state = admitted(ButtonVisualState::Up, clip);
        first_state.opacity = 1.0;
        let mut first = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![0; 8],
        };
        assert_eq!(
            draw_authenticated_sprite_pixels_impl(&mut first, &atlas, 2, geometry, first_state),
            Ok(2)
        );
        assert_eq!(first.pixels, [255, 0, 0, 255, 191, 0, 64, 255]);

        let mut second_state = first_state;
        second_state.frame = FrameSelection {
            one_based_frame: 2,
            zero_based_index: 1,
        };
        let mut second = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![0; 8],
        };
        assert_eq!(
            draw_authenticated_sprite_pixels_impl(&mut second, &atlas, 2, geometry, second_state),
            Ok(2)
        );
        assert_eq!(second.pixels, [64, 0, 191, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn authenticated_pixel_path_applies_effect_clip_opacity_and_fixed_blend() {
        let source = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![200, 100, 50, 128],
        };
        let mut up = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![20, 40, 60, 255, 20, 40, 60, 255],
        };
        let mut disabled = up.clone();
        let clip = ClipRect {
            left: 1,
            top: 0,
            right: 2,
            bottom: 1,
        };
        let geometry = AuthenticatedSpriteGeometry {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            border: None,
        };
        assert_eq!(
            draw_authenticated_sprite_pixels_impl(
                &mut up,
                &source,
                1,
                geometry,
                admitted(ButtonVisualState::Up, clip),
            ),
            Ok(1)
        );
        assert_eq!(
            draw_authenticated_sprite_pixels_impl(
                &mut disabled,
                &source,
                1,
                geometry,
                admitted(ButtonVisualState::Disable, clip),
            ),
            Ok(1)
        );
        assert_eq!(&up.pixels[..4], &[20, 40, 60, 255]);
        assert_eq!(&disabled.pixels[..4], &[20, 40, 60, 255]);
        assert_eq!(&up.pixels[4..], &[65, 55, 57, 64]);
        assert_eq!(&disabled.pixels[4..], &[45, 59, 74, 64]);
    }
}
