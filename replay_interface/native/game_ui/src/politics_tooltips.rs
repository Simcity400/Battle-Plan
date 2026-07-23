//! Retained A24/PTI1 semantic-inspection binding.
//!
//! Production XUI11 pixels are replayed by the authenticated selected-Present
//! UI pass, while the exact currently visible tooltip is carried by d72/TTI2.
//! Country politics state comes from d14/d15. Consequently d73/PTI1 is not a
//! production dependency. The API remains available for retained recordings
//! and forensic inspection; it never evaluates Clausewitz triggers, recomputes
//! modifier aggregates, derives advisor eligibility, or invents a missing row.

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
