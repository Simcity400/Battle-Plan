//! Exact UI03 visibility, graphical traversal, and positional-animation laws.
//!
//! The GUI object visible bit and the graphical traversal bytes are separate
//! state domains. Show/hide bridges them; Render2dTree evaluates the graphical
//! state and does not reread the GUI-object byte.

pub const GUI_VISIBLE_MASK: u8 = 0x08;

/// Exact logical visibility read at `CGuiObject + 0x75`.
pub fn gui_object_is_visible(flags_at_0x75: u8) -> bool {
    flags_at_0x75 & GUI_VISIBLE_MASK != 0
}

/// Exact shared state-writer operation. All unrelated bits are preserved.
pub fn write_gui_object_visible(flags_at_0x75: u8, visible: bool) -> u8 {
    (flags_at_0x75 & !GUI_VISIBLE_MASK) | (u8::from(visible) << 3)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicalRootVisibility {
    /// Mirrors `CGraphicalObject + 0x138`.
    pub descendants_enabled: bool,
}

/// Container show/hide propagates the same state to both graphical roots.
pub fn propagate_container_visibility(
    gui_flags_at_0x75: &mut u8,
    roots: &mut [GraphicalRootVisibility; 2],
    visible: bool,
) {
    *gui_flags_at_0x75 = write_gui_object_visible(*gui_flags_at_0x75, visible);
    for root in roots {
        root.descendants_enabled = visible;
    }
}

/// Fixed-window show/hide propagates to its single graphical root.
pub fn propagate_fixed_window_visibility(
    gui_flags_at_0x75: &mut u8,
    root: &mut GraphicalRootVisibility,
    visible: bool,
) {
    *gui_flags_at_0x75 = write_gui_object_visible(*gui_flags_at_0x75, visible);
    root.descendants_enabled = visible;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicalNodeState {
    /// Mirrors `CGraphicalObject + 0x154 != 0`.
    pub locally_renderable: bool,
    /// Mirrors `CGraphicalObject + 0x138 != 0`.
    pub descendants_enabled: bool,
    /// Mirrors the child count at `CGraphicalObject + 0xdc`.
    pub child_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Render2dDecision {
    /// Evaluator bit `0x01`: append this node to the primary 2D list.
    pub append_primary: bool,
    /// Evaluator bit `0x08`: traverse the child range at `+0xd0`.
    pub traverse_descendants: bool,
}

impl Render2dDecision {
    pub fn is_pruned_leaf(self) -> bool {
        !self.append_primary && !self.traverse_descendants
    }
}

/// Exact predicate result from the Render2dTree node evaluator.
///
/// This function intentionally does not synthesize draw ordering. It supplies
/// the proven append/recurse flags to the compositor's established traversal.
pub fn evaluate_render2d_node(state: GraphicalNodeState) -> Render2dDecision {
    Render2dDecision {
        append_primary: state.locally_renderable,
        traverse_descendants: state.descendants_enabled && state.child_count != 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionAnimationType {
    Accelerated,
    Decelerated,
    Smoothstep,
    Linear,
}

impl PositionAnimationType {
    pub fn authored_code(self) -> u32 {
        match self {
            Self::Accelerated => 0,
            Self::Decelerated => 1,
            Self::Smoothstep => 2,
            Self::Linear => 3,
        }
    }

    pub fn runtime_code(self) -> u32 {
        self.authored_code() + 1
    }

    /// Exact parser mapping; every unrecognized value defaults to decelerated.
    pub fn parse_authored(value: &str) -> Self {
        match value {
            "accelerated" => Self::Accelerated,
            "smoothstep" => Self::Smoothstep,
            "linear" => Self::Linear,
            "decelerated" | _ => Self::Decelerated,
        }
    }
}

/// Runtime code zero means inactive; codes one through four are authored+1.
pub fn animation_type_from_runtime_code(code: u32) -> Option<PositionAnimationType> {
    match code {
        1 => Some(PositionAnimationType::Accelerated),
        2 => Some(PositionAnimationType::Decelerated),
        3 => Some(PositionAnimationType::Smoothstep),
        4 => Some(PositionAnimationType::Linear),
        _ => None,
    }
}

fn smoothstep_once(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    (3.0 - 2.0 * value) * value * value
}

/// Exact easing dispatch for the active animation enum.
pub fn eased_position_progress(kind: PositionAnimationType, normalized_time: f64) -> f64 {
    match kind {
        PositionAnimationType::Accelerated => normalized_time.powi(4),
        PositionAnimationType::Decelerated => (1.0 - (1.0 + 4.0 * normalized_time).powi(-2)) / 0.96,
        PositionAnimationType::Smoothstep => smoothstep_once(smoothstep_once(normalized_time)),
        PositionAnimationType::Linear => normalized_time,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionAxis {
    pub target: i32,
    /// Exact initializer value: `target - current`.
    pub delta: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionAnimationState {
    /// Zero is inactive; one through four select the exact easing laws.
    pub runtime_code: u32,
    pub elapsed: f64,
    pub duration: f64,
    pub axes: [PositionAxis; 2],
    /// Hide is applied only when an animated hide reaches completion.
    pub deferred_hide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionAnimationSample {
    pub elapsed: f64,
    pub position: [i32; 2],
    pub complete: bool,
    pub perform_deferred_hide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionAnimationError {
    InvalidRuntimeCode,
    NonFiniteTime,
    NegativeElapsedOrDelta,
    NonPositiveDuration,
    CoordinateOverflow,
}

fn exact_target(axes: [PositionAxis; 2]) -> [i32; 2] {
    [axes[0].target, axes[1].target]
}

/// Advance the exact updater by one non-negative frame delta.
///
/// Completion is deliberately strict (`elapsed > duration`), not `>=`. At
/// completion the exact target is written and deferred hide is performed.
pub fn advance_position_animation(
    state: PositionAnimationState,
    dt: f64,
) -> Result<PositionAnimationSample, PositionAnimationError> {
    if !state.elapsed.is_finite() || !state.duration.is_finite() || !dt.is_finite() {
        return Err(PositionAnimationError::NonFiniteTime);
    }
    if state.elapsed < 0.0 || dt < 0.0 {
        return Err(PositionAnimationError::NegativeElapsedOrDelta);
    }
    if state.runtime_code == 0 {
        return Ok(PositionAnimationSample {
            elapsed: state.elapsed,
            position: exact_target(state.axes),
            complete: true,
            perform_deferred_hide: state.deferred_hide,
        });
    }
    let kind = animation_type_from_runtime_code(state.runtime_code)
        .ok_or(PositionAnimationError::InvalidRuntimeCode)?;
    if state.duration <= 0.0 {
        // Zero-duration show/hide uses the immediate path before this updater.
        return Err(PositionAnimationError::NonPositiveDuration);
    }

    let elapsed = state.elapsed + dt;
    if !elapsed.is_finite() {
        return Err(PositionAnimationError::NonFiniteTime);
    }
    if elapsed > state.duration {
        return Ok(PositionAnimationSample {
            elapsed,
            position: exact_target(state.axes),
            complete: true,
            perform_deferred_hide: state.deferred_hide,
        });
    }

    let eased = eased_position_progress(kind, elapsed / state.duration);
    let mut position = [0_i32; 2];
    for (index, axis) in state.axes.iter().enumerate() {
        let remaining = ((1.0 - eased) * f64::from(axis.delta)).trunc();
        if !remaining.is_finite() || remaining < i64::MIN as f64 || remaining > i64::MAX as f64 {
            return Err(PositionAnimationError::CoordinateOverflow);
        }
        let value = i64::from(axis.target) - remaining as i64;
        position[index] =
            i32::try_from(value).map_err(|_| PositionAnimationError::CoordinateOverflow)?;
    }
    Ok(PositionAnimationSample {
        elapsed,
        position,
        complete: false,
        perform_deferred_hide: false,
    })
}
