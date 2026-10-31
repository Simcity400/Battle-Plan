//! Native military-aid list attribution shared by map and dashboard readers.
use crate::rows::{DivisionRow, SentVolunteerRow};

/// Source owner used by the native division census; host remains roster owner.
pub fn owner(division: &DivisionRow) -> u32 {
    if division.expeditionary_owner as i32 > 0 {
        division.expeditionary_owner
    } else {
        division.roster_owner
    }
}

/// Volunteer attribution follows the live division's logical owner, not
/// roster, expeditionary source or current province ownership.
pub fn volunteer_host(division: &DivisionRow) -> u32 {
    division.logical_owner
}

/// Only one alias may hold the sender's list. A complete empty table proves
/// an empty list, while missing positions refuse the list rather than its tail.
pub fn volunteer_list<'a>(
    lists: impl IntoIterator<Item = &'a [SentVolunteerRow]>,
) -> Result<&'a [SentVolunteerRow], String> {
    let mut lists = lists.into_iter().filter(|list| !list.is_empty());
    let list = lists.next().unwrap_or_default();
    if lists.next().is_some() {
        return Err("SentVolunteers holds two lists for the sender".into());
    }
    if list
        .iter()
        .enumerate()
        .any(|(at, row)| row.ordinal as usize != at)
    {
        return Err("SentVolunteers ordinals are not contiguous".into());
    }
    Ok(list)
}

/// Null and nonlive references are native skips, not unknown divisions.
pub fn live_division(divisions: &[DivisionRow], reference: u64) -> Option<&DivisionRow> {
    if reference == 0 {
        return None;
    }
    divisions
        .binary_search_by_key(&reference, |row| row.id)
        .ok()
        .map(|at| &divisions[at])
}

/// The deployed-manpower loop keeps repeated references and signed wrapping.
/// Exile eligibility and sender contribution remain independent demanded reads.
pub fn listed_manpower(
    references: impl IntoIterator<Item = u64>,
    divisions: &[DivisionRow],
    ordinary: impl FnMut(&DivisionRow) -> Option<bool>,
    contribution: impl FnMut(u64) -> Option<i32>,
) -> Option<i32> {
    listed_manpower_with(
        references,
        |reference| live_division(divisions, reference),
        ordinary,
        contribution,
    )
}

/// [`listed_manpower`] with `live` finding a live division by its reference.
pub fn listed_manpower_with<D>(
    references: impl IntoIterator<Item = u64>,
    mut live: impl FnMut(u64) -> Option<D>,
    mut ordinary: impl FnMut(D) -> Option<bool>,
    mut contribution: impl FnMut(u64) -> Option<i32>,
) -> Option<i32> {
    let mut total = 0_i32;
    for reference in references {
        // Null references are native skips, as nonlive ones are.
        if reference != 0
            && let Some(division) = live(reference)
            && ordinary(division)?
        {
            total = total.wrapping_add(contribution(reference)?);
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_repeats_null_stale_and_wrapping_keep_native_scope() {
        let rows = [SentVolunteerRow {
            country: 91,
            ordinal: 0,
            division: 7,
        }];
        assert_eq!(volunteer_list([&[][..], &rows]).unwrap(), &rows);
        assert!(volunteer_list([&rows[..], &rows[..]]).is_err());
        let divisions = [DivisionRow {
            id: 7,
            expeditionary_owner: u32::MAX,
            roster_owner: 91,
            ..Default::default()
        }];
        assert_eq!(owner(&divisions[0]), 91);
        for (raw, expected) in [
            (0, 91),
            (17, 17),
            (i32::MAX as u32, i32::MAX as u32),
            (1 << 31, 91),
        ] {
            let mut division = divisions[0].clone();
            division.expeditionary_owner = raw;
            assert_eq!(owner(&division), expected);
        }
        assert_eq!(
            listed_manpower(
                [7, 7, 0, 99],
                &divisions,
                |_| Some(true),
                |_| Some(i32::MAX)
            ),
            Some(-2)
        );
        assert_eq!(
            listed_manpower([7], &divisions, |_| None, |_| Some(0)),
            None
        );
        assert_eq!(
            listed_manpower([7], &divisions, |_| Some(false), |_| None),
            Some(0)
        );
        assert_eq!(
            listed_manpower(
                [0, 99],
                &divisions,
                |_| panic!("native skip"),
                |_| panic!("native skip")
            ),
            Some(0)
        );
        assert_eq!(
            listed_manpower([7], &divisions, |_| Some(true), |_| None),
            None
        );
    }
}
