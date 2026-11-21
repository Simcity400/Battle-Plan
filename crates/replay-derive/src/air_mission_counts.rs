
use replay_data::rows::{AirMissionCountRow, AirRegionalMissionRow, AirWingRow};
use std::collections::{BTreeMap, HashMap};

pub const MISSIONS: usize = AirMissionCountRow::MISSIONS;

pub const MISSION_ENTRY_TEMPLATE: &str = "air_mission_mapicon_entry";
pub type Counts = [i32; MISSIONS];
pub type RegionalWings<'a> = BTreeMap<u32, Vec<&'a AirWingRow>>;
pub type RegionalCounts = BTreeMap<u32, Counts>;
pub type OwnerRegions<'a> = BTreeMap<u32, (RegionalCounts, RegionalWings<'a>)>;

pub fn count(counts: &mut Counts, wing: &AirWingRow) {
    count_of(counts, wing.planes, wing.mission_mask);
}

pub fn count_of(counts: &mut Counts, planes: i32, mission_mask: u32) {
    AirMissionCountRow::count(counts, mission_mask, planes);
}

pub fn registration(
    wing: u64,
    row: Option<(u32, Option<u32>)>,
    ours: u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<Option<u32>, String> {
    let (owner, region) =
        row.ok_or_else(|| format!("registered wing {wing} has no AirWings row"))?;
    let owner =
        slot(owner).ok_or_else(|| format!("registered wing {wing} has an unresolved owner"))?;
    if owner != ours {
        return Ok(None);
    }
    region
        .map(Some)
        .ok_or_else(|| format!("registered wing {wing} has no mission region"))
}

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
        let found = wings.get(&row.wing);
        let Some(region) = registration(
            row.wing,
            found.map(|wing| (wing.owner, wing.mission_region)),
            ours,
            &slot,
        )?
        else {
            continue;
        };
        let wing = found.expect("a counted registration has its wing");
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
