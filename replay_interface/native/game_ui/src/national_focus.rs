use crate::research_canvas::{
    admit_research_canvas, AdmissionRejection, ResearchCanvasKind, ResearchCanvasRequest,
};
use replay_core::{
    FocusView, Leaf, NationalFocusPresentationExactView, ViewStatus,
};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_FOCUS_ICON_SPRITE: &str = "GFX_goal_unknown";
pub const STANDARD_FOCUS_UNAVAILABLE_SPRITE: &str = "GFX_focus_unavailable";
pub const STANDARD_FOCUS_AVAILABLE_SPRITE: &str = "GFX_focus_can_start";
pub const STANDARD_FOCUS_CURRENT_SPRITE: &str = "GFX_focus_current";
pub const STANDARD_FOCUS_COMPLETED_SPRITE: &str = "GFX_focus_completed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDefinitionKind {
    Local,
    Shared,
    Joint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusStateBackgroundSprites {
    pub unavailable: String,
    pub available: String,
    pub current: String,
    pub completed: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusNodeDefinition {
    pub id: String,
    pub kind: FocusDefinitionKind,
    pub icon_sprite: Option<String>,
    /// Exact optional FILE-authored overlay token. `None` is authored-empty.
    pub overlay_sprite: Option<String>,
    pub title_loc_key: String,
    /// Required for joint focuses because FILE authors multiple participant-specific
    /// background families. Local/shared focuses use the four frozen standard sprites.
    pub joint_state_backgrounds: Option<FocusStateBackgroundSprites>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusCatalog {
    pub nodes: BTreeMap<String, FocusNodeDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusTreeTopology {
    pub tree_id: String,
    /// UI10's exact local plus imported shared/joint component.
    pub effective_node_ids: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusNodeState {
    Completed,
    Current,
    Available,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusNodePresentation {
    pub id: String,
    pub definition_kind: FocusDefinitionKind,
    pub state: FocusNodeState,
    pub background_sprite: String,
    pub icon_sprite: String,
    pub overlay_sprite: Option<String>,
    pub title_loc_key: String,
    /// Exact A31 leaves. `None` means the d89 sidecar was not used; `Some(false)`
    /// is an authenticated hidden result and must not be conflated with absence.
    pub display_progress_percent: Option<u8>,
    pub continuous_glow_visible: Option<bool>,
    pub highlight_glow_visible: Option<bool>,
    pub historical_marker_visible: Option<bool>,
    pub joint_viewing_country_tag: Option<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedNationalFocusPresentation {
    pub tree_id: String,
    pub current_national: Option<String>,
    pub current_continuous: Option<String>,
    pub completed: Vec<String>,
    pub available: Vec<String>,
    pub progress_days: Option<f64>,
    pub paused: bool,
    /// Exact producer-selected visible sequence, ready for the admitted UI09 canvas.
    pub nodes: Vec<FocusNodePresentation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusAdmissionCode {
    MissingMeasuredState,
    InvalidSet,
    TopologyMismatch,
    MissingDefinition,
    InvalidIcon,
    InvalidBackground,
    MissingVisibleText,
    Canvas,
    ExactPresentation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusAdmissionRejection {
    pub code: FocusAdmissionCode,
    pub reason: String,
    pub canvas_rejection: Option<AdmissionRejection>,
}

fn reject(code: FocusAdmissionCode, reason: impl Into<String>) -> FocusAdmissionRejection {
    FocusAdmissionRejection {
        code,
        reason: reason.into(),
        canvas_rejection: None,
    }
}

fn measured<'a, T>(leaf: &'a Leaf<T>, field: &str) -> Result<&'a T, FocusAdmissionRejection> {
    match leaf {
        Leaf::Measured(value) => Ok(value),
        Leaf::Absent | Leaf::Unobtained => Err(reject(
            FocusAdmissionCode::MissingMeasuredState,
            format!("focus.{field} is not measured"),
        )),
    }
}

fn unique_nonempty(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .all(|value| !value.trim().is_empty() && seen.insert(value.as_str()))
}

fn validate_legacy_current(
    legacy: &Leaf<String>,
    exact_set: &[String],
    field: &str,
) -> Result<(), FocusAdmissionRejection> {
    if let Leaf::Measured(value) = legacy {
        if exact_set != [value.clone()] {
            return Err(reject(
                FocusAdmissionCode::InvalidSet,
                format!("legacy {field} disagrees with its measured-empty exact set"),
            ));
        }
    }
    Ok(())
}

fn background_sprite(
    def: &FocusNodeDefinition,
    state: FocusNodeState,
) -> Result<String, FocusAdmissionRejection> {
    let value = match def.kind {
        FocusDefinitionKind::Local | FocusDefinitionKind::Shared => match state {
            FocusNodeState::Completed => STANDARD_FOCUS_COMPLETED_SPRITE,
            FocusNodeState::Current => STANDARD_FOCUS_CURRENT_SPRITE,
            FocusNodeState::Available => STANDARD_FOCUS_AVAILABLE_SPRITE,
            FocusNodeState::Unavailable => STANDARD_FOCUS_UNAVAILABLE_SPRITE,
        },
        FocusDefinitionKind::Joint => {
            let sprites = def.joint_state_backgrounds.as_ref().ok_or_else(|| {
                reject(
                    FocusAdmissionCode::InvalidBackground,
                    format!(
                        "joint focus {} lacks an exact authored background family",
                        def.id
                    ),
                )
            })?;
            match state {
                FocusNodeState::Completed => sprites.completed.as_str(),
                FocusNodeState::Current => sprites.current.as_str(),
                FocusNodeState::Available => sprites.available.as_str(),
                FocusNodeState::Unavailable => sprites.unavailable.as_str(),
            }
        }
    };
    if value.trim().is_empty() {
        return Err(reject(
            FocusAdmissionCode::InvalidBackground,
            format!("focus {} carries an empty state background token", def.id),
        ));
    }
    Ok(value.to_string())
}

pub fn admit_national_focus_presentation(
    focus: &FocusView,
    topology: &FocusTreeTopology,
    catalog: &FocusCatalog,
    canvas: &ResearchCanvasRequest,
) -> Result<AdmittedNationalFocusPresentation, FocusAdmissionRejection> {
    let tree_id = measured(&focus.focus_tree, "focus_tree")?;
    let current = measured(&focus.current_set, "current_set")?;
    let current_continuous = measured(&focus.current_continuous_set, "current_continuous_set")?;
    let completed = measured(&focus.completed, "completed")?;
    let completed_count = measured(&focus.completed_count, "completed_count")?;
    let available = measured(&focus.available, "available")?;
    let visible_order = measured(&focus.visible_order, "visible_order")?;
    let paused = *measured(&focus.paused, "paused")?;

    if tree_id != &topology.tree_id {
        return Err(reject(
            FocusAdmissionCode::TopologyMismatch,
            "runtime focus_tree does not match the selected UI10 topology",
        ));
    }
    if current.len() > 1
        || current_continuous.len() > 1
        || (!current.is_empty() && !current_continuous.is_empty())
        || !unique_nonempty(current)
        || !unique_nonempty(current_continuous)
        || !unique_nonempty(completed)
        || !unique_nonempty(available)
        || !unique_nonempty(visible_order)
        || *completed_count < 0
        || *completed_count as usize != completed.len()
    {
        return Err(reject(
            FocusAdmissionCode::InvalidSet,
            "focus current/completed/available/visible sets violate cardinality, uniqueness, or count",
        ));
    }
    validate_legacy_current(&focus.current, current, "current")?;
    validate_legacy_current(
        &focus.current_continuous,
        current_continuous,
        "current_continuous",
    )?;

    let visible: BTreeSet<&str> = visible_order.iter().map(String::as_str).collect();
    if visible_order
        .iter()
        .any(|id| !topology.effective_node_ids.contains(id))
        || current
            .iter()
            .any(|id| !topology.effective_node_ids.contains(id))
        || available.iter().any(|id| !visible.contains(id.as_str()))
    {
        return Err(reject(
            FocusAdmissionCode::TopologyMismatch,
            "visible/current/available focus ids do not belong to the selected effective topology",
        ));
    }

    let completed_set: BTreeSet<&str> = completed.iter().map(String::as_str).collect();
    let current_set: BTreeSet<&str> = current.iter().map(String::as_str).collect();
    let available_set: BTreeSet<&str> = available.iter().map(String::as_str).collect();
    if completed_set.iter().any(|id| available_set.contains(id))
        || completed_set.iter().any(|id| current_set.contains(id))
    {
        return Err(reject(
            FocusAdmissionCode::InvalidSet,
            "completed focuses cannot also be current or selectable",
        ));
    }

    for id in completed.iter().chain(visible_order) {
        if !catalog.nodes.contains_key(id) {
            return Err(reject(
                FocusAdmissionCode::MissingDefinition,
                format!("focus definition {id} is absent from the effective catalog"),
            ));
        }
    }

    let progress_days = if current.is_empty() && current_continuous.is_empty() {
        if matches!(focus.progress, Leaf::Measured(_)) {
            return Err(reject(
                FocusAdmissionCode::InvalidSet,
                "progress is measured while both exact current sets are empty",
            ));
        }
        None
    } else {
        let value = *measured(&focus.progress, "progress")?;
        if !value.is_finite() || value < 0.0 {
            return Err(reject(
                FocusAdmissionCode::InvalidSet,
                "focus progress days must be finite and non-negative",
            ));
        }
        Some(value)
    };

    let canvas_focus = match canvas {
        ResearchCanvasRequest::Focus(value) if value.node_ids == *visible_order => value,
        ResearchCanvasRequest::Focus(_) => {
            return Err(reject(
                FocusAdmissionCode::TopologyMismatch,
                "UI09 focus canvas node sequence differs from focus.visible_order",
            ));
        }
        _ => {
            return Err(reject(
                FocusAdmissionCode::Canvas,
                "national-focus presentation requires a UI09 focus canvas request",
            ));
        }
    };

    let mut nodes = Vec::with_capacity(visible_order.len());
    for id in visible_order {
        let def = &catalog.nodes[id];
        let icon_sprite = match def.icon_sprite.as_deref() {
            Some(value) if !value.trim().is_empty() => value.to_string(),
            Some(_) => {
                return Err(reject(
                    FocusAdmissionCode::InvalidIcon,
                    format!("focus {id} carries an empty authored icon token"),
                ));
            }
            None => DEFAULT_FOCUS_ICON_SPRITE.to_string(),
        };
        let overlay_sprite = match def.overlay_sprite.as_deref() {
            Some(value) if !value.trim().is_empty() => Some(value.to_string()),
            Some(_) => {
                return Err(reject(
                    FocusAdmissionCode::InvalidIcon,
                    format!("focus {id} carries an empty authored overlay token"),
                ));
            }
            None => None,
        };
        if def.id != *id {
            return Err(reject(
                FocusAdmissionCode::MissingDefinition,
                format!(
                    "focus catalog key {id} disagrees with definition id {}",
                    def.id
                ),
            ));
        }
        if !canvas_focus.text.required_keys.contains(&def.title_loc_key) {
            return Err(reject(
                FocusAdmissionCode::MissingVisibleText,
                format!("focus {id} title key is absent from the UI09/XUI06 text witness"),
            ));
        }
        let state = if completed_set.contains(id.as_str()) {
            FocusNodeState::Completed
        } else if current_set.contains(id.as_str()) {
            FocusNodeState::Current
        } else if available_set.contains(id.as_str()) {
            FocusNodeState::Available
        } else {
            FocusNodeState::Unavailable
        };
        let background_sprite = background_sprite(def, state)?;
        nodes.push(FocusNodePresentation {
            id: id.clone(),
            definition_kind: def.kind,
            state,
            background_sprite,
            icon_sprite,
            overlay_sprite,
            title_loc_key: def.title_loc_key.clone(),
            display_progress_percent: None,
            continuous_glow_visible: None,
            highlight_glow_visible: None,
            historical_marker_visible: None,
            joint_viewing_country_tag: None,
        });
    }

    let admitted = admit_research_canvas(canvas).map_err(|error| FocusAdmissionRejection {
        code: FocusAdmissionCode::Canvas,
        reason: format!("UI09 focus canvas rejected: {}", error.reason),
        canvas_rejection: Some(error),
    })?;
    if admitted.kind() != ResearchCanvasKind::Focus {
        return Err(reject(
            FocusAdmissionCode::Canvas,
            "UI09 admitted a non-focus canvas for national-focus presentation",
        ));
    }

    Ok(AdmittedNationalFocusPresentation {
        tree_id: tree_id.clone(),
        current_national: current.first().cloned(),
        current_continuous: current_continuous.first().cloned(),
        completed: completed.clone(),
        available: available.clone(),
        progress_days,
        paused,
        nodes,
    })
}

/// Production A31/XUI10 admission. The legacy D12/D45 state remains useful for
/// tree/current/completed context, but every final per-item presentation leaf
/// and engine order must agree with the exact, hour-bound d89 sidecar.
pub fn admit_exact_national_focus_presentation(
    focus: &FocusView,
    exact: &NationalFocusPresentationExactView,
    country_tag: &str,
    topology: &FocusTreeTopology,
    catalog: &FocusCatalog,
    canvas: &ResearchCanvasRequest,
) -> Result<AdmittedNationalFocusPresentation, FocusAdmissionRejection> {
    if exact.status != ViewStatus::Present {
        return Err(reject(
            FocusAdmissionCode::ExactPresentation,
            "national-focus d89 presentation is absent or unavailable",
        ));
    }
    let presentation = exact.presentation.as_ref().ok_or_else(|| {
        reject(
            FocusAdmissionCode::ExactPresentation,
            "national-focus d89 present status omitted its typed presentation",
        )
    })?;
    if presentation.country_tag != country_tag {
        return Err(reject(
            FocusAdmissionCode::ExactPresentation,
            "national-focus d89 country differs from selected replay country",
        ));
    }

    let mut admitted = admit_national_focus_presentation(focus, topology, catalog, canvas)?;
    if presentation.items.len() != admitted.nodes.len()
        || presentation.expected_visible_item_count as usize != admitted.nodes.len()
    {
        return Err(reject(
            FocusAdmissionCode::ExactPresentation,
            "national-focus d89 item count differs from the admitted canvas",
        ));
    }
    for (ordinal, (node, item)) in admitted
        .nodes
        .iter_mut()
        .zip(&presentation.items)
        .enumerate()
    {
        let exact_state = match item.presentation_state_code {
            0 => FocusNodeState::Unavailable,
            1 => FocusNodeState::Completed,
            2 => FocusNodeState::Available,
            3 => FocusNodeState::Current,
            _ => {
                return Err(reject(
                    FocusAdmissionCode::ExactPresentation,
                    "national-focus d89 state code escaped typed replay validation",
                ));
            }
        };
        if item.source_ordinal as usize != ordinal
            || !portable_focus_id(&item.focus_id)
            || !portable_focus_id(&node.id)
            || item.focus_id != node.id
            || exact_state != node.state
        {
            return Err(reject(
                FocusAdmissionCode::ExactPresentation,
                format!(
                    "national-focus d89 item {ordinal} disagrees with D12/D45/UI09 identity or state"
                ),
            ));
        }
        node.display_progress_percent = Some(item.display_progress_percent);
        node.continuous_glow_visible = Some(item.continuous_glow_visible);
        node.highlight_glow_visible = Some(item.highlight_glow_visible);
        node.historical_marker_visible = Some(item.historical_marker_visible);
        node.joint_viewing_country_tag = Some(item.joint_viewing_country_tag.clone());
    }
    Ok(admitted)
}

fn portable_focus_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 128
        || !(bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
        })
        || value.contains("..")
    {
        return false;
    }
    !value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .is_some_and(|digits| {
            digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod portable_identity_tests {
    use super::portable_focus_id;

    #[test]
    fn ui_boundary_preserves_mod_ids_and_rejects_pointer_path_mutations() {
        for accepted in ["GER_autobahn", "mod-focus.v2", "shared@branch"] {
            assert!(portable_focus_id(accepted), "{accepted}");
        }
        for rejected in [
            "0x000001abcdef1234",
            r"C:\\hoi4\\focus",
            "/opt/hoi4/focus",
            r"\\host\\share\\focus",
            "focus/child",
            "focus child",
        ] {
            assert!(!portable_focus_id(rejected), "{rejected}");
        }
    }
}
