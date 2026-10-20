//! Regional registration counts, independent of presentation and effective country.
//! Registration order is significant: a negative total restarts before the next
//! wrapping addition. A signed nonpositive mission mask admits no mission.

use replay_data::rows::{AirRegionalMissionRow, AirWingRow};
use std::collections::{BTreeMap, HashMap};

/// The supported native mission-bit domain, including unnamed kinds.
pub const MISSIONS: usize = 18;

/// The regional map's per-mission GUI template. Its `button` names the mission
/// strip sprite and draws mission bit `b` at the strip's one-based frame `b + 1`.
pub const MISSION_ENTRY_TEMPLATE: &str = "air_mission_mapicon_entry";
pub type Counts = [i32; MISSIONS];
pub type RegionalWings<'a> = BTreeMap<u32, Vec<&'a AirWingRow>>;
pub type RegionalCounts = BTreeMap<u32, Counts>;
pub type OwnerRegions<'a> = BTreeMap<u32, (RegionalCounts, RegionalWings<'a>)>;

/// Selected bits receive the wing's whole aircraft count, including zero.
pub fn count(counts: &mut Counts, wing: &AirWingRow) {
    if (wing.mission_mask as i32) <= 0 {
        return;
    }
    for (bit, total) in counts.iter_mut().enumerate() {
        if wing.mission_mask & (1 << bit) != 0 {
            *total = (*total).max(0).wrapping_add(wing.planes);
        }
    }
}

/// Read all owners with one wing index and one stable registration sort.
/// Occurrences remain in each region's roster; callers count unique handles
/// separately, never by adding mission columns.
pub fn owners<'a>(
    registrations: &[AirRegionalMissionRow],
    wings: &'a [AirWingRow],
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<OwnerRegions<'a>, String> {
    let wings: HashMap<_, _> = wings.iter().map(|wing| (wing.id, wing)).collect();
    let mut registrations: Vec<_> = registrations.iter().collect();
    registrations.sort_by_key(|row| row.ordinal);
    let mut result = OwnerRegions::new();
    for row in registrations {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| format!("registered wing {} has no AirWings row", row.wing))?;
        let owner = slot(wing.owner)
            .ok_or_else(|| format!("registered wing {} has an unresolved owner", wing.id))?;
        let region = wing
            .mission_region
            .ok_or_else(|| format!("registered wing {} has no mission region", wing.id))?;
        let (counts, roster) = result.entry(owner).or_default();
        count(counts.entry(region).or_insert([-1; MISSIONS]), wing);
        roster.entry(region).or_default().push(*wing);
    }
    Ok(result)
}

/// The map's selected-owner projection. It preserves the original demand
/// order: resolve every registration's owner, but ask for a region only for ours.
pub fn regional_counts<'a>(
    registrations: &[AirRegionalMissionRow],
    wings: &'a [AirWingRow],
    ours: u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<(RegionalCounts, RegionalWings<'a>), String> {
    let wings: HashMap<_, _> = wings.iter().map(|wing| (wing.id, wing)).collect();
    let mut registrations: Vec<_> = registrations.iter().collect();
    registrations.sort_by_key(|row| row.ordinal);
    let mut regions = BTreeMap::new();
    let mut regional_wings = RegionalWings::new();
    for row in registrations {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| format!("registered wing {} has no AirWings row", row.wing))?;
        let owner = slot(wing.owner)
            .ok_or_else(|| format!("registered wing {} has an unresolved owner", wing.id))?;
        if owner != ours {
            continue;
        }
        let region = wing
            .mission_region
            .ok_or_else(|| format!("registered wing {} has no mission region", wing.id))?;
        count(regions.entry(region).or_insert([-1; MISSIONS]), wing);
        regional_wings.entry(region).or_default().push(*wing);
    }
    Ok((regions, regional_wings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_masks_restart_and_wrap() {
        let mut totals = [-1; MISSIONS];
        for planes in [i32::MAX, 1] {
            count(
                &mut totals,
                &AirWingRow {
                    planes,
                    mission_mask: 5,
                    ..Default::default()
                },
            );
        }
        assert_eq!(totals[0], i32::MIN);
        assert_eq!(totals[2], i32::MIN);
        count(
            &mut totals,
            &AirWingRow {
                planes: 9,
                mission_mask: 1,
                ..Default::default()
            },
        );
        assert_eq!(totals[0], 9);
        count(
            &mut totals,
            &AirWingRow {
                planes: 99,
                mission_mask: 0x8000_0001,
                ..Default::default()
            },
        );
        assert_eq!(totals[0], 9);
    }
    #[test]
    fn foreign_region_is_not_demanded_by_selected_owner() {
        let wings = [AirWingRow {
            id: 1,
            owner: 2,
            ..Default::default()
        }];
        let rows = [AirRegionalMissionRow {
            wing: 1,
            ..Default::default()
        }];
        assert!(
            regional_counts(&rows, &wings, 1, Some)
                .unwrap()
                .0
                .is_empty()
        );
        assert!(owners(&rows, &wings, Some).is_err());
    }
}
