//! Opt-in observations of the shipping desktop consumers and finalized Present.
//! This module creates no controls, pixels, admitted state or input capability.
//! Missing observations remain null/named; diagnostic failure cannot alter a
//! product transaction. The harness binds this child/stream to exact file bytes.
use crate::app::ProductMapCompositionReceipt;
use crate::observer::PovPicker;
use crate::transport::Transport;
use crate::ui_runtime::{ObserverUiOutcome, ProductObserverUiRuntime};
use crate::win32::WindowEvent;
use game_ui::compose::Rect;
use game_ui::production_renderer::ObserverViewerUiState;
use game_ui::stored_country_selection::StoredCountrySelection;
use game_ui::window_manager_law::{
    AuthenticatedReplayTooltipOwner, ReplayPointerTarget, ReplayWindowManagerSnapshot,
};
use serde_json::{json, Value};
use std::io::Write;

pub(crate) struct DesktopDiagnostics {
    enabled: bool,
    input_seq: u64,
    present_seq: u64,
}

pub(crate) struct InputObservation {
    row: Value,
}

impl InputObservation {
    pub(crate) fn action(&mut self, route: &str, changed: bool) {
        self.row["route"] = json!(route);
        self.row["disposition"] = json!(if changed { "applied" } else { "unchanged" });
    }

    pub(crate) fn pointer(&mut self, target: Option<&ReplayPointerTarget>) {
        self.row["route"] = json!("authored_pointer_move");
        self.row["disposition"] = json!(if target.is_some() {
            "applied"
        } else {
            "no_admitted_target"
        });
        self.row["target"] = target.map(pointer_json).unwrap_or(Value::Null);
    }
}

/// Copied only with diagnostics enabled, before app feedback moves outcome
/// fields. The selected spelling still requires the actual WorldFrame below.
pub(crate) struct UiOutcomeObservation {
    selected: StoredCountrySelection,
    panel_open: bool,
    panel_available: bool,
    panel_country: Option<String>,
    semantic_receipts: Vec<String>,
    feature_local_gaps: Vec<String>,
    gaps_debug: String,
}

impl DesktopDiagnostics {
    pub(crate) fn from_environment() -> Self {
        Self::with_enabled(
            std::env::var_os("REPLAY_DESKTOP_DIAGNOSTICS").as_deref()
                == Some(std::ffi::OsStr::new("1")),
        )
    }

    pub(crate) fn with_enabled(enabled: bool) -> Self {
        Self {
            enabled,
            input_seq: 0,
            present_seq: 0,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn begin_input(
        &mut self,
        event: &WindowEvent,
        runtime: &ProductObserverUiRuntime,
    ) -> Option<InputObservation> {
        if !self.enabled {
            return None;
        }
        let mut row = match event {
            WindowEvent::KeyDown { vk, shift } => {
                json!({"kind":"key_down", "vk":vk, "shift":shift})
            }
            WindowEvent::MouseMove { x, y } => json!({"kind":"mouse_move", "x":x, "y":y}),
            _ => return None,
        };
        let Some(next) = self.input_seq.checked_add(1) else {
            self.failed(
                "diagnostic_input_sequence_overflow",
                "no input receipt can be issued",
            );
            self.enabled = false;
            return None;
        };
        self.input_seq = next;
        let snapshot = runtime.desktop_snapshot();
        row["schema"] = json!("hoi4.viewer.desktop.input.v1");
        row["pid"] = json!(std::process::id());
        row["input_seq"] = json!(next);
        row["generation"] = json!(snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.as_ref())
            .map(|s| s.committed_frame_generation));
        row["generation_unavailable"] = json!(snapshot.err().map(|e| e.to_string()));
        row["route"] = json!("unhandled_or_higher_owner");
        row["disposition"] = json!("not_applied_by_inspected_consumer");
        row["target"] = Value::Null;
        Some(InputObservation { row })
    }

    pub(crate) fn finish_input(&self, input: Option<InputObservation>) {
        if let Some(input) = input {
            emit(&input.row);
        }
    }

    pub(crate) fn capture_outcome(
        &self,
        outcome: &ObserverUiOutcome,
    ) -> Option<UiOutcomeObservation> {
        self.enabled.then(|| UiOutcomeObservation {
            selected: outcome.selected_country.clone(),
            panel_open: outcome.production_lines_open,
            panel_available: outcome.production_lines_available,
            panel_country: outcome.production_lines_country.clone(),
            semantic_receipts: outcome.semantic_receipts.clone(),
            feature_local_gaps: outcome.feature_local_gaps.clone(),
            gaps_debug: format!("{:?}", outcome.gaps),
        })
    }

    pub(crate) fn present(&mut self, receipt: Option<&ProductMapCompositionReceipt>) {
        if let Some(row) = self.present_row(receipt) {
            emit(&row);
        }
    }

    pub(crate) fn present_row(
        &mut self,
        receipt: Option<&ProductMapCompositionReceipt>,
    ) -> Option<Value> {
        if !self.enabled {
            return None;
        }
        let Some(next) = self.present_seq.checked_add(1) else {
            self.failed(
                "diagnostic_present_sequence_overflow",
                "no presentation receipt can be issued",
            );
            self.enabled = false;
            return None;
        };
        self.present_seq = next;
        Some(
            json!({"schema":"hoi4.viewer.desktop.present.v1", "pid":std::process::id(),
            "present_seq":next, "last_input_seq":self.input_seq, "map":receipt}),
        )
    }

    pub(crate) fn failed(&self, disposition: &str, reason: &str) {
        if let Some(row) = self.failure_row(disposition, reason) {
            emit(&row);
        }
    }

    pub(crate) fn failure_row(&self, disposition: &str, reason: &str) -> Option<Value> {
        if !self.enabled {
            return None;
        }
        Some(
            json!({"schema":"hoi4.viewer.desktop.present.v1", "pid":std::process::id(),
            "present_seq":self.present_seq, "last_input_seq":self.input_seq,
            "map":null, "disposition":disposition, "reason":reason}),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn presented_ui(
        &self,
        map: Option<&ProductMapCompositionReceipt>,
        snapshot: Option<&ReplayWindowManagerSnapshot>,
        outcome: Option<&UiOutcomeObservation>,
        world: Option<&replay_core::WorldFrame>,
        picker: &PovPicker,
        transport: &Transport,
        runtime: &ProductObserverUiRuntime,
        viewer: &ObserverViewerUiState,
        hwnd: winapi::shared::windef::HWND,
        pending_tooltip: Option<&(AuthenticatedReplayTooltipOwner, String)>,
        higher_app_owner: impl Fn(i32, i32) -> bool,
    ) {
        if !self.enabled {
            return;
        }
        let generation = snapshot.map(|s| s.committed_frame_generation);
        let mut gaps = Vec::<String>::new();
        let controls = match (snapshot, map) {
            (Some(snapshot), Some(map)) => project_controls(
                snapshot,
                [map.viewport_width, map.viewport_height],
                |x, y| {
                    if higher_app_owner(x, y) {
                        return Ok(None);
                    }
                    runtime
                        .desktop_pointer_target(viewer, x, y)
                        .map_err(|e| e.to_string())
                },
                &mut gaps,
            ),
            _ => {
                gaps.push("committed_control_geometry_unavailable".into());
                Vec::new()
            }
        };
        let selected = outcome
            .and_then(|o| world.and_then(|w| o.selected.named_in(w)))
            .map(|n| n.tag());
        let (choices, choices_gap) = match picker {
            PovPicker::Available { choices, .. } => (
                Some(
                    choices
                        .iter()
                        .map(|c| world.and_then(|w| w.named_country(&c.tag)).map(|n| n.tag()))
                        .collect::<Vec<_>>(),
                ),
                None,
            ),
            PovPicker::Unavailable(reason) => (None, Some(reason)),
        };
        let semantic = outcome.map(|o| o.semantic_receipts.as_slice());
        let panel_semantic = semantic.map(|rows| {
            rows.iter()
                .filter(|s| s.starts_with("XUI12:"))
                .collect::<Vec<_>>()
        });
        let panel_country = outcome
            .and_then(|o| o.panel_country.as_deref())
            .and_then(|tag| world.and_then(|w| w.named_country(tag)))
            .map(|n| n.tag());
        let tooltip = project_tooltip(
            pending_tooltip,
            snapshot,
            semantic,
            || {
                runtime
                    .authenticated_authored_tooltip_owner_at_cursor()
                    .map_err(|e| e.to_string())
            },
            &mut gaps,
        );
        emit(
            &json!({"schema":"hoi4.viewer.desktop.ui.v1", "pid":std::process::id(),
            "present_seq":self.present_seq, "map":map, "generation":generation,
            "input_transition_generation":snapshot.map(|s| s.dispatched_input_transition_generation),
            "input_receipt_schema":"hoi4.viewer.desktop.input.v1", "last_input_seq":self.input_seq,
            "playing":transport.is_playing(), "transport_index":transport.current_index(),
            "window_dpi":window_dpi(hwnd), "selected_country":selected, "country_choices":choices,
            "country_choices_unavailable":choices_gap, "country_choices_unnameable":picker.unnameable(),
            "production_lines":{"open":outcome.map(|o|o.panel_open), "available":outcome.map(|o|o.panel_available),
                "country":panel_country, "semantic_receipts":panel_semantic,
                "gaps":outcome.map(|o|o.feature_local_gaps.iter().filter(|s|s.starts_with("XUI12:")).collect::<Vec<_>>())},
            "controls":controls, "tooltip":tooltip, "diagnostic_gaps":gaps,
            "semantic_receipts":semantic, "feature_local_gaps":outcome.map(|o|&o.feature_local_gaps),
            "gaps_debug":outcome.map(|o|&o.gaps_debug), "snapshot_debug":snapshot.map(|s|format!("{s:?}"))}),
        );
    }
}

/// This observation is called only after Present and the existing owner commit.
/// The pending candidate came from the real successful tooltip blend; neither a
/// current hover nor an unrelated semantic receipt proves that it was drawn.
pub(crate) fn project_tooltip(
    pending: Option<&(AuthenticatedReplayTooltipOwner, String)>,
    snapshot: Option<&ReplayWindowManagerSnapshot>,
    semantic: Option<&[String]>,
    current_owner: impl FnOnce() -> Result<Option<AuthenticatedReplayTooltipOwner>, String>,
    gaps: &mut Vec<String>,
) -> Value {
    let (Some((candidate, record)), Some(snapshot), Some(semantic)) = (pending, snapshot, semantic)
    else {
        return Value::Null;
    };
    match current_owner() {
        Ok(Some(current))
            if current == *candidate
                && snapshot.tooltip.as_ref().is_some_and(|tip| {
                    tip.window_id == current.window_id()
                        && tip.widget_id == current.element_id()
                        && tip.tooltip_instance_record == *record
                }) =>
        {
            let receipts = semantic
                .iter()
                .filter(|s| {
                    s.starts_with("XUI05:rows=") && s.ends_with(":manager-overlay-composited-last")
                })
                .collect::<Vec<_>>();
            if receipts.is_empty() {
                gaps.push("tooltip_blend_receipt_unavailable".into());
                Value::Null
            } else {
                tooltip_json(&current, &receipts)
            }
        }
        Ok(_) => {
            gaps.push("presented_tooltip_owner_differs_from_blended_candidate".into());
            Value::Null
        }
        Err(error) => {
            gaps.push(format!("tooltip_owner_unavailable:{error}"));
            Value::Null
        }
    }
}

/// The same exclusive clipped midpoint used by the harness. It proposes a
/// point; only the real consumer's hit test may establish its actual owner.
pub(crate) fn clipped_point(rect: Rect, clip: Rect, viewport: [u32; 2]) -> Option<(i32, i32)> {
    if rect.w() <= 0 || rect.h() <= 0 || clip.w() <= 0 || clip.h() <= 0 {
        return None;
    }
    let left = i64::from(rect.x()).max(i64::from(clip.x())).max(0);
    let top = i64::from(rect.y()).max(i64::from(clip.y())).max(0);
    let right = (i64::from(rect.x()) + i64::from(rect.w()))
        .min(i64::from(clip.x()) + i64::from(clip.w()))
        .min(i64::from(viewport[0]));
    let bottom = (i64::from(rect.y()) + i64::from(rect.h()))
        .min(i64::from(clip.y()) + i64::from(clip.h()))
        .min(i64::from(viewport[1]));
    if right <= left || bottom <= top {
        return None;
    }
    let x = i16::try_from((left + right - 1) / 2).ok()?;
    let y = i16::try_from((top + bottom - 1) / 2).ok()?;
    Some((i32::from(x), i32::from(y)))
}

pub(crate) fn project_controls(
    snapshot: &ReplayWindowManagerSnapshot,
    viewport: [u32; 2],
    mut actual_target: impl FnMut(i32, i32) -> Result<Option<ReplayPointerTarget>, String>,
    gaps: &mut Vec<String>,
) -> Vec<Value> {
    let mut controls = Vec::new();
    for window in &snapshot.stack {
        for element in &window.element_interactions {
            if !element.visible {
                continue;
            }
            let (Some(rect), Some(clip)) = (element.screen_rect, element.clip_rect) else {
                continue;
            };
            let Some((x, y)) = clipped_point(rect, clip, viewport) else {
                continue;
            };
            match actual_target(x, y) {
                Ok(Some(target))
                    if target.window_id == element.window_id
                        && target.runtime_instance_token == element.runtime_instance_token
                        && target.source_file == element.source_file
                        && target.surface_id == element.surface_id
                        && target.element_id == element.element_id
                        && target.child_path == element.child_path
                        && target.screen_rect == rect
                        && target.clip_rect == clip =>
                {
                    let mut control = pointer_json(&target);
                    control["generation"] = json!(snapshot.committed_frame_generation);
                    control["visible"] = json!(element.visible);
                    control["owns_pointer"] = json!(true);
                    control["coordinate_space"] = json!("win32_client_lparam");
                    control["modal_owner"] = json!(window.modal_owner);
                    controls.push(control);
                }
                Ok(_) => {}
                Err(reason) => gaps.push(format!(
                    "control:{}:{}:{reason}",
                    element.window_id, element.element_id
                )),
            }
        }
    }
    controls
}

fn rect_json(rect: Rect) -> [i32; 4] {
    [rect.x(), rect.y(), rect.w(), rect.h()]
}

pub(crate) fn pointer_json(target: &ReplayPointerTarget) -> Value {
    json!({"window_id":target.window_id,"element_id":target.element_id,
        "runtime_instance_token":target.runtime_instance_token,"source_file":target.source_file,
        "surface_id":target.surface_id,"child_path":target.child_path,
        "client_rect":rect_json(target.screen_rect),"clip_rect":rect_json(target.clip_rect),
        "global_root_sibling":target.global_root_sibling,"stack_ordinal":target.stack_ordinal,
        "paint_ordinal":target.paint_ordinal,"disposition_debug":format!("{:?}",target.disposition),
        "visual_enabled":target.visual_enabled,"authored_name":target.authored_name,"element_type":target.element_type})
}

fn tooltip_json(owner: &AuthenticatedReplayTooltipOwner, receipts: &[&String]) -> Value {
    json!({"window_id":owner.window_id(),"widget_id":owner.element_id(),
        "runtime_instance_token":owner.runtime_instance_token(),"source_file":owner.source_file(),
        "surface_id":owner.surface_id(),"child_path":owner.child_path(),
        "generation":owner.frame_generation(),"presented":true,"semantic_receipts":receipts,
        "engine_instance_token":owner.engine_instance_token(),"client_rect":rect_json(owner.screen_rect()),
        "clip_rect":rect_json(owner.clip_rect()),"stack_ordinal":owner.stack_ordinal(),"paint_ordinal":owner.paint_ordinal()})
}

fn emit(row: &Value) {
    // A broken diagnostic pipe is not a product admission/transaction failure.
    // Its absent/truncated receipt cannot pass the external bounded validator.
    let _ = writeln!(std::io::stderr().lock(), "{row}");
}

fn window_dpi(hwnd: winapi::shared::windef::HWND) -> Option<u32> {
    #[link(name = "user32")]
    extern "system" {
        fn GetDpiForWindow(hwnd: winapi::shared::windef::HWND) -> u32;
    }
    std::num::NonZeroU32::new(unsafe { GetDpiForWindow(hwnd) }).map(|dpi| dpi.get())
}
