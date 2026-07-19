//! XUI07 runtime sprite/frame, shader-state, clipping, opacity, and blend laws.
//!
//! Built-in shading covers only byte-bearing shipped shader laws. Other effect
//! families are admitted only with producer-resolved shaded RGBA and provenance;
//! this module never substitutes a visually tuned approximation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    pub fn one_based_frame(self, is_current_hover_target: bool) -> u32 {
        match self {
            Self::Up => 1,
            Self::Down => 2,
            Self::Over if is_current_hover_target => 3,
            Self::Over => 1,
            Self::Disable => 4,
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

/// Exact `static_button.shader` pixel law after horizontal frame selection.
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

/// Exact `buttonstate.shader` state shader after authored sprite-animation and
/// mask stages have produced `base_after_animation_and_mask`.
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

/// Exact `buttonstate_blendframes.shader` state shader.
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

/// Exact `sprite_animation.fxh::Blend` law.
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

/// Fixed-function blend state used by the shipped UI shaders:
/// source=`src_alpha`, destination=`inv_src_alpha`. The same factors are
/// applied to RGBA because the shader files declare no separate alpha factors.
pub fn blend_src_alpha_inv_src_alpha(dest: RgbaF32, source: RgbaF32) -> RgbaF32 {
    let source_factor = source.0[3];
    let dest_factor = 1.0 - source_factor;
    RgbaF32(std::array::from_fn(|index| {
        source.0[index] * source_factor + dest.0[index] * dest_factor
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingPath {
    ButtonState,
    ButtonStateBlendFrames,
    StaticButton,
    ProducerResolved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeGraphicsRequest {
    pub source_record_id: String,
    pub sprite_name: String,
    pub frame_count: usize,
    pub one_based_frame: u32,
    pub shading_path: ShadingPath,
    /// Required for effect families not covered by the built-in shader laws.
    pub producer_resolved_rgba: Option<RgbaF32>,
    pub clip: ClipRect,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeGraphicsRejection {
    MissingProvenance,
    InvalidFrame,
    InvalidClip,
    InvalidOpacity,
    MissingProducerResolvedShading,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdmittedRuntimeGraphics {
    frame: FrameSelection,
    shading_path: ShadingPath,
    clip: ClipRect,
    opacity: f32,
    producer_resolved_rgba: Option<RgbaF32>,
}

impl AdmittedRuntimeGraphics {
    pub fn frame(self) -> FrameSelection {
        self.frame
    }

    pub fn shading_path(self) -> ShadingPath {
        self.shading_path
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
    if request.shading_path == ShadingPath::ProducerResolved
        && !request.producer_resolved_rgba.is_some_and(RgbaF32::finite)
    {
        return Err(RuntimeGraphicsRejection::MissingProducerResolvedShading);
    }
    Ok(AdmittedRuntimeGraphics {
        frame,
        shading_path: request.shading_path,
        clip: request.clip,
        opacity: request.opacity,
        producer_resolved_rgba: request.producer_resolved_rgba,
    })
}
