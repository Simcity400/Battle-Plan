//! Product boundary for the authenticated A26/d72 final-tooltip blob.
//!
//! The ordinary production collector schedules d72 and passively captures the
//! exact final post-layout tooltip text and rectangle at the selected Present.
//! This includes politics tooltips. This adapter intentionally contains no
//! row reconstruction, localisation, OCR, or geometry inference.

use game_ui::hud::TooltipBinding;
use game_ui::production_renderer::tooltip_binding_from_exact_view;
use replay_core::ResolvedTooltipExactView;

pub fn binding_from_exact_view(
    view: &ResolvedTooltipExactView,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> Option<TooltipBinding> {
    tooltip_binding_from_exact_view(view, pov_tag, viewport_width, viewport_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        ResolvedTooltipExactFrame, ResolvedTooltipInstanceExact, ResolvedTooltipRectExact,
        ViewStatus,
    };

    fn present(instances: Vec<ResolvedTooltipInstanceExact>) -> ResolvedTooltipExactView {
        ResolvedTooltipExactView {
            status: ViewStatus::Present,
            frame: Some(ResolvedTooltipExactFrame {
                frame_hour: 1,
                selected_present_candidate_id: 9,
                selected_present_ordinal: 0,
                selected_present_qpc_ticks: 100,
                selected_present_receipt_sha256: "b7".repeat(32),
                producer_manifest_sha256: "a6".repeat(32),
                completeness_record_identity: "a729.tooltip.instances[1]".into(),
                instances,
            }),
        }
    }

    #[test]
    fn measured_empty_emits_no_tooltip_shell() {
        assert!(binding_from_exact_view(&present(Vec::new()), Some("GER"), 1920, 1080).is_none());
    }

    #[test]
    fn exact_blob_and_geometry_reach_hud_binding_without_row_inference() {
        let view = present(vec![ResolvedTooltipInstanceExact {
            frame_hour: 1,
            pov_country_or_global: "GER".into(),
            hovered_element_authored_name: "technology_slot".into(),
            tooltip_owner_authored_name: "ToolTip".into(),
            repeat_index: 0,
            screen_rect: ResolvedTooltipRectExact {
                x: 10,
                y: 20,
                width: 300,
                height: 100,
            },
            viewport_width_px: 1920,
            viewport_height_px: 1080,
            producer_record_identity: "controller:1".into(),
            final_resolved_text: "Exact first line\nExact second line".into(),
        }]);
        let binding =
            binding_from_exact_view(&view, Some("GER"), 1920, 1080).expect("exact tooltip");
        assert_eq!((binding.x, binding.y, binding.width, binding.height), (10, 20, 300, 100));
        assert_eq!(binding.text, "Exact first line\nExact second line");
    }

    #[test]
    fn viewport_or_pov_mismatch_fails_closed() {
        let view = present(vec![ResolvedTooltipInstanceExact {
            frame_hour: 1,
            pov_country_or_global: "GER".into(),
            hovered_element_authored_name: "technology_slot".into(),
            tooltip_owner_authored_name: "ToolTip".into(),
            repeat_index: 0,
            screen_rect: ResolvedTooltipRectExact {
                x: 10,
                y: 20,
                width: 300,
                height: 100,
            },
            viewport_width_px: 1920,
            viewport_height_px: 1080,
            producer_record_identity: "controller:1".into(),
            final_resolved_text: "Exact".into(),
        }]);
        assert!(binding_from_exact_view(&view, Some("FRA"), 1920, 1080).is_none());
        assert!(binding_from_exact_view(&view, Some("GER"), 2560, 1440).is_none());
    }
}
