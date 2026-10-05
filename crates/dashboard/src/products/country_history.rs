//! Shared country-row identity and ambiguity handling for branch histories.

use super::country_groups::HourSpan;
use crate::metadata::link_slot;
use replay::rows::CountryLinkRow;
use std::collections::HashMap;

/// Canonical country slot and native country id.
pub(crate) type Identity = (u32, u32);

pub(crate) struct SlotRows<'a, T> {
    pub rows: HashMap<u32, Option<&'a T>>,
    pub unresolved_rows: usize,
    /// Rows after the first row naming the same slot.
    pub duplicate_rows: usize,
    /// First rows that also become unknown when their slot is repeated.
    pub ambiguous_slots: usize,
}

/// No row wins when several raw links resolve to the same country slot.
pub(crate) fn unique_by_slot<'a, T>(
    rows: impl IntoIterator<Item = &'a T>,
    links: &[CountryLinkRow],
    country: impl Fn(&T) -> u32,
) -> SlotRows<'a, T> {
    let mut result = SlotRows {
        rows: HashMap::new(),
        unresolved_rows: 0,
        duplicate_rows: 0,
        ambiguous_slots: 0,
    };
    for row in rows {
        let Some(slot) = link_slot(links, country(row)) else {
            result.unresolved_rows += 1;
            continue;
        };
        result
            .rows
            .entry(slot)
            .and_modify(|seen| {
                result.duplicate_rows += 1;
                result.ambiguous_slots += usize::from(seen.is_some());
                *seen = None;
            })
            .or_insert(Some(row));
    }
    result
}

/// Return whether this hour extends the preceding identity segment. Missing
/// hours and a different native country under the tag always start a new one.
pub(crate) fn observe_identity(
    segments: &mut Vec<HourSpan>,
    last_identity: &mut Option<(Identity, i32)>,
    identity: Identity,
    hour: i32,
) -> bool {
    let contiguous =
        last_identity.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
    if contiguous {
        segments.last_mut().expect("identity segment").last_hour = hour;
    } else {
        segments.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
    *last_identity = Some((identity, hour));
    contiguous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_history_refuses_every_ambiguous_row_and_unresolved_link() {
        let links = [
            CountryLinkRow {
                id: 1,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 8,
            },
        ];
        let rows = [1, 2, 1, 3, 99];
        let result = unique_by_slot(&rows, &links, |country| *country);
        assert_eq!(result.rows[&7], None);
        assert_eq!(result.rows[&8], Some(&3));
        assert_eq!(result.unresolved_rows, 1);
        assert_eq!(result.duplicate_rows, 2);
        assert_eq!(result.ambiguous_slots, 1);
    }

    #[test]
    fn country_history_identity_segments_break_on_gaps_and_native_id_changes() {
        let mut segments = Vec::new();
        let mut last = None;
        assert!(!observe_identity(&mut segments, &mut last, (7, 1), 10));
        assert!(observe_identity(&mut segments, &mut last, (7, 1), 11));
        assert!(!observe_identity(&mut segments, &mut last, (7, 1), 13));
        assert!(!observe_identity(&mut segments, &mut last, (7, 2), 14));
        assert!(!observe_identity(&mut segments, &mut last, (8, 2), 15));
        assert_eq!(
            segments
                .iter()
                .map(|span| (span.first_hour, span.last_hour))
                .collect::<Vec<_>>(),
            [(10, 11), (13, 13), (14, 14), (15, 15)]
        );
        assert!(!observe_identity(
            &mut segments,
            &mut last,
            (8, 2),
            i32::MAX
        ));
        assert!(!observe_identity(
            &mut segments,
            &mut last,
            (8, 2),
            i32::MIN
        ));
    }
}
