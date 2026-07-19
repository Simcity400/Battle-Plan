//! XUI04 widget visual-state admission law.
//!
//! This layer joins the exact UI02 button state carrier, authored sprite-frame
//! counts, UI01 attention events, UI06/UI07 drag snapshots, and shipped progress
//! shader predicates.  It deliberately does not infer selected/active/toggle
//! frames from sprite names or strip order: those compound states require an
//! explicit producer-resolved one-based frame.

use crate::runtime_graphics::{select_one_based_frame, ButtonVisualState, FrameSelection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetFamily {
    Window,
    Container,
    Icon,
    Button,
    GuiButton,
    Checkbox,
    DropDown,
    EditBox,
    Browser,
    InstantText,
    Text,
    List,
    SmoothList,
    Grid,
    Overlap,
    LegacyScrollbar,
    ExtendedScrollbar,
    ScrollbarPart,
}

impl WidgetFamily {
    pub fn from_element_type(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "windowtype" => Some(Self::Window),
            "containerwindowtype" => Some(Self::Container),
            "icontype" => Some(Self::Icon),
            "buttontype" => Some(Self::Button),
            "guibuttontype" => Some(Self::GuiButton),
            "checkboxtype" => Some(Self::Checkbox),
            "dropdownboxtype" => Some(Self::DropDown),
            "editboxtype" => Some(Self::EditBox),
            "browsertype" => Some(Self::Browser),
            "instanttextboxtype" => Some(Self::InstantText),
            "textboxtype" => Some(Self::Text),
            "listboxtype" => Some(Self::List),
            "smoothlistboxtype" => Some(Self::SmoothList),
            "gridboxtype" => Some(Self::Grid),
            "overlappingelementsboxtype" => Some(Self::Overlap),
            "scrollbartype" => Some(Self::LegacyScrollbar),
            "extendedscrollbartype" => Some(Self::ExtendedScrollbar),
            "decreasebutton" | "increasebutton" | "expandbutton" | "track" | "slider" => {
                Some(Self::ScrollbarPart)
            }
            _ => None,
        }
    }

    pub fn has_sprite_surface(self) -> bool {
        matches!(
            self,
            Self::Icon
                | Self::Button
                | Self::GuiButton
                | Self::Checkbox
                | Self::DropDown
                | Self::ScrollbarPart
        )
    }

    fn standard_button_carrier(self) -> bool {
        matches!(self, Self::Button | Self::GuiButton | Self::ScrollbarPart)
    }

    fn scrollbar(self) -> bool {
        matches!(self, Self::LegacyScrollbar | Self::ExtendedScrollbar)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyPresentation {
    /// Non-interactive display object.
    Passive,
    /// Replay-only navigation/filter/selection control; no game mutation.
    Navigation,
    /// Preserve the recorded visual snapshot but suppress all activation.
    InertSnapshot,
    /// Render the exact disabled state and suppress all activation.
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionCarrier {
    /// UI01 threshold event targeting `active_goal/goal_icon`.
    PoliticalGoalPulse { trigger_event: bool },
    /// A producer-resolved attention state whose pixels/frame are separately
    /// provenance-bound (for families without a universal attention law).
    ProducerResolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DragSnapshot {
    pub source_record_id: String,
    pub axis: ScrollbarAxis,
    pub value: f32,
    pub thumb_x: i32,
    pub thumb_y: i32,
    pub pointer_x: i32,
    pub pointer_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressEffect {
    Linear,
    StartEnd,
    Reverse,
    MinMaxColor,
    MinMaxTexture,
    Radial,
    Circular,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressFill {
    pub effect: ProgressEffect,
    /// Shipped `CurrentState`; ignored only by `Circular`.
    pub current_state: f32,
    /// Shipped circular vertex carrier `vTextureToRender.x`; required only by
    /// `Circular` and preserved without clamping.
    pub circular_mix: Option<f32>,
}

impl ProgressFill {
    pub fn valid(self) -> bool {
        self.current_state.is_finite()
            && match self.effect {
                ProgressEffect::Circular => self.circular_mix.is_some_and(f32::is_finite),
                _ => self.circular_mix.is_none(),
            }
    }

    /// Exact foreground/first-texture predicate for the non-interpolating
    /// shipped progress effects. Radial and Circular return `None` because they
    /// blend instead of choosing one texture.
    pub fn foreground_at(self, u: f32) -> Option<bool> {
        match self.effect {
            ProgressEffect::Linear | ProgressEffect::StartEnd => Some(u <= self.current_state),
            ProgressEffect::Reverse => Some(u >= 1.0 - self.current_state),
            ProgressEffect::MinMaxColor => {
                Some((u <= 0.5 && u <= self.current_state) || (u >= 0.5 && u >= self.current_state))
            }
            ProgressEffect::MinMaxTexture => {
                Some((u < 0.5 && u >= self.current_state) || (u >= 0.5 && u <= self.current_state))
            }
            ProgressEffect::Radial | ProgressEffect::Circular => None,
        }
    }

    /// Exact texture-two lerp factor for the shipped radial/circular effects.
    pub fn texture_two_mix(self, u: f32, v: f32) -> Option<f32> {
        match self.effect {
            ProgressEffect::Radial => {
                let dx = 0.5_f32 - u;
                let dy = 0.5_f32 - v;
                let angle = dx.atan2(dy) + std::f32::consts::PI;
                Some(
                    ((angle - self.current_state * std::f32::consts::PI * 2.0) * 50.0)
                        .clamp(0.0, 1.0),
                )
            }
            ProgressEffect::Circular => self.circular_mix,
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetVisualRequest {
    pub source_record_id: String,
    pub family: WidgetFamily,
    pub presentation: ReadOnlyPresentation,
    pub interaction: Option<ButtonVisualState>,
    pub is_current_hover_target: bool,
    pub frame_count: usize,
    /// Producer-resolved one-based frame. Mandatory for selected, active, and
    /// toggle states because no universal bool-to-frame mapping is authored.
    pub one_based_frame: Option<u32>,
    pub selected: bool,
    pub active: bool,
    pub toggle: Option<bool>,
    pub attention: Option<AttentionCarrier>,
    pub progress: Option<ProgressFill>,
    pub drag: Option<DragSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetVisualRejection {
    MissingProvenance,
    InvalidFrame,
    MissingButtonState,
    UnsupportedButtonState,
    MissingSemanticFrame,
    InvalidProgress,
    ProgressOnNonSprite,
    DragOnNonScrollbar,
    InvalidDragSnapshot,
    DisabledStateMismatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedWidgetVisual {
    frame: Option<FrameSelection>,
    pub progress: Option<ProgressFill>,
    pub drag: Option<DragSnapshot>,
    pub attention: Option<AttentionCarrier>,
    pub allows_navigation_events: bool,
    pub allows_game_mutation: bool,
}

impl AdmittedWidgetVisual {
    pub fn frame(&self) -> Option<FrameSelection> {
        self.frame
    }
}

pub fn admit_widget_visual(
    request: &WidgetVisualRequest,
) -> Result<AdmittedWidgetVisual, WidgetVisualRejection> {
    if request.source_record_id.trim().is_empty() {
        return Err(WidgetVisualRejection::MissingProvenance);
    }

    let semantic_state = request.selected || request.active || request.toggle.is_some();
    if semantic_state && request.one_based_frame.is_none() {
        return Err(WidgetVisualRejection::MissingSemanticFrame);
    }

    if request.presentation == ReadOnlyPresentation::Disabled {
        if request.family.standard_button_carrier() {
            if request.interaction != Some(ButtonVisualState::Disable) {
                return Err(WidgetVisualRejection::DisabledStateMismatch);
            }
        } else if request.family.has_sprite_surface() && request.one_based_frame.is_none() {
            return Err(WidgetVisualRejection::DisabledStateMismatch);
        }
    }

    let frame = if let Some(one_based) = request.one_based_frame {
        Some(
            select_one_based_frame(one_based, request.frame_count)
                .ok_or(WidgetVisualRejection::InvalidFrame)?,
        )
    } else if request.family.standard_button_carrier() {
        let interaction = request
            .interaction
            .ok_or(WidgetVisualRejection::MissingButtonState)?;
        Some(
            select_one_based_frame(
                interaction.one_based_frame(request.is_current_hover_target),
                request.frame_count,
            )
            .ok_or(WidgetVisualRejection::InvalidFrame)?,
        )
    } else if request.family.has_sprite_surface() {
        Some(
            select_one_based_frame(1, request.frame_count)
                .ok_or(WidgetVisualRejection::InvalidFrame)?,
        )
    } else {
        if request.interaction.is_some() {
            return Err(WidgetVisualRejection::UnsupportedButtonState);
        }
        None
    };

    if let Some(progress) = request.progress {
        if !request.family.has_sprite_surface() {
            return Err(WidgetVisualRejection::ProgressOnNonSprite);
        }
        if !progress.valid() {
            return Err(WidgetVisualRejection::InvalidProgress);
        }
    }

    if let Some(drag) = &request.drag {
        if !request.family.scrollbar() {
            return Err(WidgetVisualRejection::DragOnNonScrollbar);
        }
        if drag.source_record_id.trim().is_empty() || !drag.value.is_finite() {
            return Err(WidgetVisualRejection::InvalidDragSnapshot);
        }
    }

    Ok(AdmittedWidgetVisual {
        frame,
        progress: request.progress,
        drag: request.drag.clone(),
        attention: request.attention,
        allows_navigation_events: request.presentation == ReadOnlyPresentation::Navigation,
        // The replay surface is categorically read-only. Even navigation
        // controls may only mutate viewer state, never the recorded game.
        allows_game_mutation: false,
    })
}
