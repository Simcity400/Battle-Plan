//! Product boundary for extractor-resolved tooltip instances.

use game_ui::tooltip_compositor::{
    compose_tooltip, ResolvedPlacement, ScreenRect, TooltipDrawCommand, TooltipPayload,
};
use game_ui::{GameUiError, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTooltipInstance {
    pub placement: Option<ResolvedPlacement>,
    pub payload: Option<TooltipPayload>,
}

pub fn compose_resolved_instance(
    instance: &ResolvedTooltipInstance,
    viewport: ScreenRect,
) -> Result<Vec<TooltipDrawCommand>> {
    let placement = instance.placement.as_ref().ok_or_else(|| {
        GameUiError::missing("XUI05 DATA_BLOCKED: resolved tooltip screen rectangle")
    })?;
    let payload = instance.payload.as_ref().ok_or_else(|| {
        GameUiError::missing("XUI05 DATA_BLOCKED: ordered resolved tooltip content rows")
    })?;
    compose_tooltip(placement, viewport, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_ui::tooltip_compositor::{PlacementEvidence, TooltipOwner};

    #[test]
    fn missing_upstream_leaf_fails_closed() {
        let error = compose_resolved_instance(
            &ResolvedTooltipInstance {
                placement: None,
                payload: None,
            },
            ScreenRect::new(0, 0, 1920, 1080),
        )
        .unwrap_err();
        assert!(error.to_string().contains("DATA_BLOCKED"));
    }

    #[test]
    fn complete_exact_instance_reaches_product_compositor() {
        let result = compose_resolved_instance(
            &ResolvedTooltipInstance {
                placement: Some(ResolvedPlacement {
                    screen_rect: ScreenRect::new(10, 20, 300, 100),
                    evidence: PlacementEvidence::ExtractedEngineRect {
                        producer_record: "tooltip_instances[0]".into(),
                    },
                }),
                payload: Some(TooltipPayload {
                    owner: TooltipOwner {
                        element_path: "topbar/political_power".into(),
                        repeat_index: 0,
                    },
                    rows: vec![],
                }),
            },
            ScreenRect::new(0, 0, 1920, 1080),
        );
        assert!(result.is_ok());
    }
}
