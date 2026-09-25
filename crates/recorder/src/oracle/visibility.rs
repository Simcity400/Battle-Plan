//! Fog-rebuild inputs (L2b): RelationLinks against the saved relations that set
//! its slots, and named limitations for the inputs no save serializer writes.

use std::{collections::BTreeMap, io};

use engine::{cair_base_access_relation, cattache_relation, coffer_air_base_access_relation};
use installed_content::save::Save;
use replay::{
    frame::DecodedFrame,
    rows::{RelationLinkRow, tables},
};

use super::{CountryContext, Report, TokenNames, complete, invalid, required_country_tag};

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report.attempt("visibility RelationLinks", |report| {
        compare_links(frame, save, countries, tokens, report)
    });
    complete_comparison &=
        report.attempt("visibility inputs", |report| record_inputs(frame, report));
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "fog-rebuild input comparison incomplete; see per-table errors",
        ))
    }
}

/// `CAttacheRelation::apply_link@140D12DA0` sets the second country's status
/// toward the first at `+0x210`; `CAirBaseAccessRelation` and
/// `COfferAirBaseAccessRelation` (`14196DE50`, `14196DF30`) set the first
/// country's status toward the second at `+0x2C0` through `140D18680`, and
/// `clear_link` removes them, so each saved relation of those types is one set
/// slot. A relation is saved under both endpoints; the slot is counted once.
pub(super) fn compare_links(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::RelationLinks>(frame)?;
    let spelling = |token: u32| {
        tokens.get(&token).copied().ok_or_else(|| {
            invalid(format!(
                "the recorded lexical dictionary has no spelling for relation type {token:#x}"
            ))
        })
    };
    let attache = spelling(cattache_relation::TYPE)?;
    let air = [
        spelling(cair_base_access_relation::TYPE)?,
        spelling(coffer_air_base_access_relation::TYPE)?,
    ];
    let mut expected: BTreeMap<(&[u8], &[u8]), u8> = BTreeMap::new();
    for relation in &save.relation_tuples {
        let kind = relation.kind.as_slice();
        let (key, bit) = if kind == attache {
            (
                (relation.second.as_slice(), relation.first.as_slice()),
                RelationLinkRow::ATTACHE,
            )
        } else if air.contains(&kind) {
            (
                (relation.first.as_slice(), relation.second.as_slice()),
                RelationLinkRow::AIR_BASE_ACCESS,
            )
        } else {
            continue;
        };
        *expected.entry(key).or_default() |= bit;
    }
    let by_slot: BTreeMap<u32, &[u8]> = countries
        .by_tag
        .iter()
        .map(|(&tag, country)| (country.canonical_slot, tag))
        .collect();
    let mut actual: BTreeMap<(&[u8], &[u8]), u8> = BTreeMap::new();
    for row in rows {
        let country = required_country_tag(row.country, &countries.links)?;
        let target = by_slot.get(&row.target_index).copied().ok_or_else(|| {
            invalid(format!(
                "RelationLinks target slot {} has no captured country",
                row.target_index
            ))
        })?;
        *actual.entry((country, target)).or_default() |= row.links;
    }
    let mut compared = 0;
    for (&(country, target), &saved) in &expected {
        let captured = actual.remove(&(country, target)).unwrap_or(0);
        compared += 1;
        if captured != saved {
            report.difference(
                format!(
                    "RelationLinks {} -> {}",
                    String::from_utf8_lossy(country),
                    String::from_utf8_lossy(target)
                ),
                captured,
                saved,
            );
        }
    }
    for ((country, target), links) in actual {
        report.difference(
            format!(
                "RelationLinks {} -> {}",
                String::from_utf8_lossy(country),
                String::from_utf8_lossy(target)
            ),
            links,
            "no saved relation",
        );
    }
    report.compared("RelationLinks", compared);
    Ok(())
}

/// Inputs the text save does not carry: they decode, but nothing certifies them.
fn record_inputs(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    for (table, rows, why) in [
        (
            "SimulationFeatures",
            complete::<tables::SimulationFeatures>(frame)?.len(),
            "the DLC feature mask is the running session's, not a saved field",
        ),
        (
            "CountryVisibility",
            complete::<tables::CountryVisibility>(frame)?.len(),
            "the viewer modifier 0x24B is a computed country modifier the save does not write",
        ),
        (
            "StateVisibility",
            complete::<tables::StateVisibility>(frame)?.len(),
            "the state modifier 0x4B is a computed state modifier the save does not write",
        ),
        (
            "BuildingDetection",
            complete::<tables::BuildingDetection>(frame)?.len(),
            "no inspected serializer writes the raid-target status detection bits",
        ),
    ] {
        report.compared(table, 0);
        report.limitation(
            &format!("{why}, so this table has no save comparison; the recorded rows decode but are not certified against a save"),
            format!("{table}: {rows} decoded rows, zero save comparisons"),
        );
    }
    Ok(())
}
