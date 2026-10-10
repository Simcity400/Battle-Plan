//! Recorded operative facts shared by roster and placement products.

use crate::{
    catalogue::{self, View},
    metadata::{link_slot, mapped_slot},
};
use installed_content::localisation::Localisation;
use replay::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, CountryRow, OperativeMembershipRow, TableId, tables},
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Frozen at a read so reused links cannot reassign an operative's country.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CountryIdentity {
    pub slot: u32,
    pub id: u32,
}

/// Build once on a read, validating the same links as every country product.
pub(crate) fn country_rows<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
) -> Option<HashMap<u32, &'a CountryRow>> {
    let (rows, unresolved) = country_rows_partial(countries, links);
    unresolved.is_empty().then_some(rows)
}

/// A per-country view can retain other valid countries while naming every
/// unresolved identity; a whole-world placement view requires all of them.
pub(crate) fn country_rows_partial<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
) -> (HashMap<u32, &'a CountryRow>, Vec<u32>) {
    let mut result = HashMap::with_capacity(countries.len());
    let mut ambiguous = HashSet::new();
    let mut unresolved = Vec::new();
    for country in countries {
        let Ok(slot) = mapped_slot(country, links) else {
            unresolved.push(country.id);
            continue;
        };
        if ambiguous.contains(&slot) {
            unresolved.push(country.id);
            continue;
        }
        if let Some(previous) = result.insert(slot, country) {
            result.remove(&slot);
            ambiguous.insert(slot);
            unresolved.push(previous.id);
            unresolved.push(country.id);
        }
    }
    (result, unresolved)
}

pub(crate) fn identity(
    raw: u32,
    links: &[CountryLinkRow],
    countries: &HashMap<u32, &CountryRow>,
) -> Option<CountryIdentity> {
    let slot = link_slot(links, raw)?;
    let row = countries.get(&slot)?;
    Some(CountryIdentity { slot, id: row.id })
}

pub(crate) fn tag_of(
    identity: Option<CountryIdentity>,
    countries: &HashMap<u32, &CountryRow>,
) -> Option<String> {
    let identity = identity?;
    let row = countries.get(&identity.slot)?;
    (row.id == identity.id).then(|| tag(row)).flatten()
}

/// Countries are codec-sorted by raw id, not by canonical slot.
pub(crate) fn country_at<'a>(
    identity: CountryIdentity,
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
) -> Option<&'a CountryRow> {
    let index = countries
        .binary_search_by_key(&identity.id, |row| row.id)
        .ok()?;
    let row = &countries[index];
    (mapped_slot(row, links).ok()? == identity.slot).then_some(row)
}

pub(crate) fn tag_at(
    identity: Option<CountryIdentity>,
    countries: &[CountryRow],
    links: &[CountryLinkRow],
) -> Option<String> {
    tag(country_at(identity?, countries, links)?)
}

fn tag(row: &CountryRow) -> Option<String> {
    std::str::from_utf8(&row.tag)
        .ok()
        .filter(|tag| !tag.is_empty() && !tag.contains('\0'))
        .map(str::to_owned)
}

/// collector::extended::intel times the residual walk itself; a held keyframe
/// or its scheduled daily slot does not establish a new membership read.
pub(crate) fn membership_read(frame: &DecodedFrame) -> bool {
    frame.metrics.table_ns[TableId::OperativeMemberships.index()] != 0
}

/// The operation/assignment walk is timed under Operations itself; held rows
/// and another table's slot timing do not establish that shared read.
pub(crate) fn operations_read(frame: &DecodedFrame) -> bool {
    frame.metrics.table_ns[TableId::Operations.index()] != 0
}

pub(crate) fn agency_feature(frame: &DecodedFrame) -> Option<bool> {
    let [features] = frame.complete::<tables::SimulationFeatures>().ok()? else {
        return None;
    };
    // Strategic operative support requires feature 0x19,
    // represented by runtime DLC-mask bit 5.
    Some(features.dlc_mask & (1 << 5) != 0)
}

/// Capture is assignment 0, independent of the capturer link.
pub(crate) fn captured(assignment: u32) -> bool {
    assignment == 0
}

pub(crate) fn supported_kind_assignment(kind: u8, assignment: u32) -> Result<(), String> {
    // The active agency vector and recruitment's three vectors supply
    // kinds 0..3. Status dispatches 0/1/3/4; terminal assignment 5
    // is identified separately.
    if kind > 3 || assignment > 5 {
        Err(format!(
            "Unsupported operative assignment {assignment} / roster {kind}"
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn mission_variant(variant: Option<u32>) -> Result<bool, String> {
    // Resetting to no mission leaves the outer assignment unchanged.
    // Mission target kind dispatches variants 1..8.
    match variant {
        None | Some(0) => Ok(false),
        Some(1..=8) => Ok(true),
        Some(variant) => Err(format!("Unsupported operative mission variant {variant}")),
    }
}

pub(crate) struct SavedName {
    pub name: Option<String>,
    pub unnamed: bool,
    pub missing: Option<String>,
    pub literal_fallback: bool,
}

pub(crate) fn saved_name(localisation: Option<&Localisation>, bytes: &[u8]) -> SavedName {
    // Saved names use the plain localiser without name generation;
    // missing keys retain their bytes.
    let literal = Localisation::default();
    let name = catalogue::localised_bytes(localisation.unwrap_or(&literal), bytes, View::Plain);
    match name {
        Ok(name) => SavedName {
            unnamed: name.is_empty(),
            name: (!name.is_empty()).then_some(name),
            missing: None,
            literal_fallback: localisation.is_none()
                && !installed_content::parser::c_string(bytes).is_empty(),
        },
        Err(reason) => SavedName {
            name: None,
            unnamed: false,
            missing: Some(reason),
            literal_fallback: false,
        },
    }
}

pub(crate) fn recorded_name(
    localisation: Option<&Localisation>,
    row: &OperativeMembershipRow,
) -> SavedName {
    match row.name.require("OperativeMemberships.name") {
        Ok(bytes) => saved_name(localisation, bytes),
        Err(reason) => SavedName {
            name: None,
            unnamed: false,
            missing: Some(reason),
            literal_fallback: false,
        },
    }
}
pub(crate) fn cooldown(row: &OperativeMembershipRow) -> Result<Option<(u32, i32, i32)>, String> {
    if row.assignment != 1 {
        return Ok(None);
    }
    let kind = *row
        .cooldown_kind
        .require("OperativeMemberships.cooldown_kind")?;
    let start = *row
        .cooldown_start_hour
        .require("OperativeMemberships.cooldown_start_hour")?;
    let end = *row
        .cooldown_end_hour
        .require("OperativeMemberships.cooldown_end_hour")?;
    match (kind, start, end) {
        (Some(kind), Some(start), Some(end)) => Ok(Some((kind, start, end))),
        _ => Err("Cooldown kind or dates were not recorded".into()),
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct MembershipSnapshot {
    row: OperativeMembershipRow,
    owner: Result<CountryIdentity, u32>,
    target: replay::rows::Recorded<Option<Result<CountryIdentity, u32>>>,
    capturer: Result<CountryIdentity, u32>,
}

impl MembershipSnapshot {
    pub(crate) fn new(
        row: &OperativeMembershipRow,
        links: &[CountryLinkRow],
        countries: &HashMap<u32, &CountryRow>,
    ) -> Self {
        let resolve = |raw| identity(raw, links, countries).ok_or(raw);
        let owner = resolve(row.owner_country);
        let target = match &row.target_country {
            replay::rows::Recorded::Value(raw) => replay::rows::Recorded::Value(raw.map(resolve)),
            replay::rows::Recorded::NotRecorded => replay::rows::Recorded::NotRecorded,
        };
        let capturer = resolve(row.capturer_country);
        let mut row = row.clone();
        // Raw aliases can name one country; roster membership is an edge,
        // not a distinct operative. Keep all other captured operands exact.
        row.owner_country = 0;
        row.roster_country = 0;
        if matches!(row.target_country, replay::rows::Recorded::Value(_)) {
            row.target_country = replay::rows::Recorded::Value(None);
        }
        row.capturer_country = 0;
        Self {
            row,
            owner,
            target,
            capturer,
        }
    }
}

/// Equal duplicate memberships count once; contradictory reads stay unknown.
pub(crate) fn admit(
    seen: &mut BTreeMap<u64, MembershipSnapshot>,
    row: &OperativeMembershipRow,
    links: &[CountryLinkRow],
    countries: &HashMap<u32, &CountryRow>,
) -> Result<bool, String> {
    let snapshot = MembershipSnapshot::new(row, links, countries);
    match seen.entry(row.id) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(snapshot);
            Ok(true)
        }
        std::collections::btree_map::Entry::Occupied(entry) if *entry.get() == snapshot => {
            Ok(false)
        }
        std::collections::btree_map::Entry::Occupied(_) => {
            Err("Conflicting operative memberships".into())
        }
    }
}

#[cfg(test)]
mod tests;
