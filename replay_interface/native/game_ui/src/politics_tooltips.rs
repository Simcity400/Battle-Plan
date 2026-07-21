//! A24/XUI11 fail-closed politics-tooltip binding.
//!
//! This module never evaluates Clausewitz triggers, recomputes modifier
//! aggregates, derives advisor eligibility from the static catalog, or
//! invents a missing runtime line. It exposes only the exact PTI1 rows admitted
//! by `replay_core` for the selected frame and politics target.

use replay_core::{Leaf, PoliticsDetailView, PoliticsTooltipInputExact};

#[derive(Debug, Clone, PartialEq)]
pub enum PoliticsTooltipSelection<'a> {
    /// The recording never carried d73/PTI1 for this frame.
    Absent,
    /// d73 was attempted, but the producer or replay admission failed closed.
    Unobtained,
    /// Authenticated rows for this exact target. An empty vector is a real
    /// measured-empty target, not an unavailable state.
    Measured(Vec<&'a PoliticsTooltipInputExact>),
}

/// Select exact evaluated rows for one politics UI target.
///
/// `target_kind` and `target_token_or_slot` are the two pointer-free producer
/// identities carried by PTI1. Callers must use the identities recorded with
/// the UI item; fuzzy token matching and fallback to authored definitions are
/// forbidden.
pub fn select_politics_tooltip_inputs<'a>(
    view: &'a PoliticsDetailView,
    target_kind: &str,
    target_token_or_slot: &str,
) -> PoliticsTooltipSelection<'a> {
    match &view.tooltip_inputs {
        Leaf::Absent => PoliticsTooltipSelection::Absent,
        Leaf::Unobtained => PoliticsTooltipSelection::Unobtained,
        Leaf::Measured(rows) => PoliticsTooltipSelection::Measured(
            rows.iter()
                .filter(|row| {
                    row.target_kind == target_kind
                        && row.target_token_or_slot == target_token_or_slot
                })
                .collect(),
        ),
    }
}

/// Preserve the producer's exact line-family order for an already selected
/// target. The extractor rejects duplicate line-family keys, so sorting by the
/// stable enum is deterministic and cannot collapse two observed rows.
pub fn ordered_politics_tooltip_inputs<'a>(
    selection: &PoliticsTooltipSelection<'a>,
) -> Option<Vec<&'a PoliticsTooltipInputExact>> {
    match selection {
        PoliticsTooltipSelection::Measured(rows) => {
            let mut ordered = rows.clone();
            ordered.sort_by_key(|row| row.line_kind);
            Some(ordered)
        }
        PoliticsTooltipSelection::Absent | PoliticsTooltipSelection::Unobtained => None,
    }
}
