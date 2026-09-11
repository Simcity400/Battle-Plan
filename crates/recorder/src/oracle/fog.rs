//! Fog-derivation input comparisons (registry domain D102).
//!
//! Three of the six families have an exact text-save representation and are
//! compared here: CUnit writer BF7010 emits `country_intel` triples, CCountry
//! writer 70C580 emits `received_air_volunteer_permission` blocks, and CTheatre
//! writer EED430 emits `unit` references plus one representative `area` province
//! per controller area. The other two are named limitations: EED430 does not
//! emit an area's whole province membership, CAirBase writer C57070 does not
//! traverse its `+0x30` country registrations at all, and no country serializer
//! path was found that writes the radar coverage vectors.

use std::{collections::BTreeMap, io};

use installed_content::save::{IntelEntry, Save};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, complete, country_tag, invalid};

/// One country's ordered host positions: the resolved host tag and its tally.
type HostPositions<'a> = Vec<(Option<&'a [u8]>, i32)>;

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report.attempt("fog UnitIntel", |report| {
        compare_unit_intel(frame, saved, report)
    });
    complete_comparison &= report.attempt("fog VolunteerHosts", |report| {
        compare_volunteer_hosts(frame, saved, links, report)
    });
    complete_comparison &= report.attempt("fog StationedUnits", |report| {
        compare_stationed(frame, saved, report)
    });
    complete_comparison &= report.attempt("fog TheatreAreas", |report| {
        compare_areas(frame, saved, report)
    });
    complete_comparison &=
        report.attempt("fog RadarCoverage", |report| record_radar(frame, report));
    complete_comparison &= report.attempt("fog AirBaseRegistrations", |report| {
        record_registrations(frame, report)
    });
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "fog derivation input comparison incomplete; see per-table errors",
        ))
    }
}

/// Ordered captured rows of one parent, rejecting a gap or a repeat in the
/// native ordinals the walker published.
fn ordered<'a, K: Ord + Copy + std::fmt::Debug, R>(
    rows: impl Iterator<Item = (K, u32, &'a R)>,
    label: &str,
) -> io::Result<BTreeMap<K, Vec<&'a R>>> {
    let mut grouped: BTreeMap<K, Vec<&R>> = BTreeMap::new();
    for (parent, ordinal, row) in rows {
        let entries = grouped.entry(parent).or_default();
        if ordinal as usize != entries.len() {
            return Err(invalid(format!(
                "captured {label} {parent:?} has a missing or repeated native ordinal {ordinal}"
            )));
        }
        entries.push(row);
    }
    Ok(grouped)
}

fn compare_unit_intel(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::UnitIntel>(frame)?
            .iter()
            .map(|row| (row.unit, row.ordinal, row)),
        "unit viewer entries",
    )?;
    let mut expected: BTreeMap<u64, &[IntelEntry]> = BTreeMap::new();
    for (id, division) in &saved.divisions {
        expected.insert(*id, &division.country_intel);
    }
    for (id, task_force) in &saved.task_forces {
        if expected.insert(*id, &task_force.country_intel).is_some() {
            return Err(invalid(
                "saved division and task force share one unit handle",
            ));
        }
    }
    let mut compared = 0;
    for (unit, entries) in &expected {
        let label = format!("unit {unit} country_intel");
        let actual = captured.get(unit).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != entries.len() {
            report.difference(format!("{label}.count"), actual.len(), entries.len());
            continue;
        }
        for (ordinal, (actual, expected)) in actual.iter().zip(entries.iter()).enumerate() {
            compared += 1;
            // The writer emits the stored key with a signed spelling; the same
            // 32 bits are compared, without normalizing either side.
            if actual.viewer != expected.viewer as u32 {
                report.difference(
                    format!("{label}[{ordinal}].viewer"),
                    actual.viewer,
                    expected.viewer,
                );
            }
            if i32::from(actual.level) != expected.level {
                report.difference(
                    format!("{label}[{ordinal}].level"),
                    actual.level,
                    expected.level,
                );
            }
        }
    }
    for unit in captured.keys() {
        if !expected.contains_key(unit) {
            report.difference(
                format!("unit {unit} country_intel"),
                "present",
                "no saved unit",
            );
        }
    }
    report.compared("UnitIntel", compared);
    report.limitation(
        "the viewer entry's native decay accumulator at +8 is not a recorded field; the save's second triple word is parsed but has no captured counterpart",
        "UnitIntel: viewer and level compare exactly, timer does not",
    );
    report.limitation(
        "a stored viewer key is a country ordinal in the engine's country array, which no table maps back to a tag; ordinals compare raw against the save's own raw word",
        "UnitIntel.viewer",
    );
    Ok(())
}

fn compare_volunteer_hosts(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::VolunteerHosts>(frame)?
            .iter()
            .map(|row| (row.country, row.position, row)),
        "volunteer host positions",
    )?;
    let mut by_tag: BTreeMap<&[u8], HostPositions<'_>> = BTreeMap::new();
    for (country, rows) in &captured {
        let tag = country_tag(*country, links)?
            .ok_or_else(|| invalid("captured volunteer host row has a zero country"))?;
        let mut positions = Vec::with_capacity(rows.len());
        for row in rows {
            positions.push((country_tag(row.host, links)?, row.deployed_divisions));
        }
        if by_tag.insert(tag, positions).is_some() {
            return Err(invalid(
                "two captured volunteer host countries resolve to one tag",
            ));
        }
    }
    let mut compared = 0;
    for (tag, country) in &saved.countries {
        let label = format!("{} received_air_volunteer_permission", show(tag));
        let actual = by_tag.remove(tag.as_slice()).unwrap_or_default();
        if actual.len() != country.volunteer_permissions.len() {
            report.difference(
                format!("{label}.count"),
                actual.len(),
                country.volunteer_permissions.len(),
            );
            continue;
        }
        for (position, (actual, expected)) in actual
            .iter()
            .zip(country.volunteer_permissions.iter())
            .enumerate()
        {
            compared += 1;
            if actual.0 != Some(expected.tag.as_slice()) {
                report.difference(
                    format!("{label}[{position}].tag"),
                    actual.0.map(show),
                    show(&expected.tag),
                );
            }
            if actual.1 != expected.count {
                report.difference(
                    format!("{label}[{position}].count"),
                    actual.1,
                    expected.count,
                );
            }
        }
    }
    for tag in by_tag.keys() {
        report.difference(
            format!("{} received_air_volunteer_permission", show(tag)),
            "present",
            "no saved country",
        );
    }
    report.compared("VolunteerHosts", compared);
    report.limitation(
        "the host vector stores raw country references and the save writes resolved tags, so two aliases of one country cannot be told apart in the comparison",
        "VolunteerHosts.host",
    );
    Ok(())
}

/// Saved volunteer theatres, keyed by handle. The writer emits
/// `volunteers_theatre` exactly when the native key is positive, which is the
/// same gate the walker applies, so the two scopes are directly comparable.
fn volunteer_theatres(
    saved: &Save,
) -> io::Result<BTreeMap<u64, &installed_content::save::naval_history::LandTheatre>> {
    let Some(countries) = &saved.naval_history.theatre_countries else {
        return Err(invalid(
            "save has no theatre roster; volunteer theatre scope cannot be established",
        ));
    };
    let mut theatres = BTreeMap::new();
    for country in countries.values() {
        for theatre in &country.land {
            if theatre.volunteers_country.is_none() {
                continue;
            }
            if theatres.insert(theatre.id, theatre).is_some() {
                return Err(invalid("saved land theatres share one handle"));
            }
        }
    }
    Ok(theatres)
}

fn compare_stationed(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::StationedUnits>(frame)?
            .iter()
            .map(|row| (row.theatre, row.ordinal, row)),
        "stationed unit positions",
    )?;
    let expected = volunteer_theatres(saved)?;
    let mut compared = 0;
    for (handle, theatre) in &expected {
        let label = format!("theatre {handle} stationed units");
        let actual = captured.get(handle).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != theatre.units.len() {
            report.difference(format!("{label}.count"), actual.len(), theatre.units.len());
            continue;
        }
        for (ordinal, (actual, expected)) in actual.iter().zip(theatre.units.iter()).enumerate() {
            compared += 1;
            if actual.unit != *expected {
                report.difference(format!("{label}[{ordinal}]"), actual.unit, expected);
            }
        }
    }
    for handle in captured.keys() {
        if !expected.contains_key(handle) {
            report.difference(
                format!("theatre {handle} stationed units"),
                "present",
                "no saved volunteer theatre",
            );
        }
    }
    report.compared("StationedUnits", compared);
    Ok(())
}

fn compare_areas(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::TheatreAreas>(frame)?
            .iter()
            .map(|row| (row.theatre, row.ordinal, row)),
        "controller area positions",
    )?;
    let expected = volunteer_theatres(saved)?;
    let mut compared = 0;
    for (handle, theatre) in &expected {
        let label = format!("theatre {handle} controller areas");
        let actual = captured.get(handle).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != theatre.area_representatives.len() {
            report.difference(
                format!("{label}.count"),
                actual.len(),
                theatre.area_representatives.len(),
            );
            continue;
        }
        for (ordinal, (actual, expected)) in actual
            .iter()
            .zip(theatre.area_representatives.iter())
            .enumerate()
        {
            compared += 1;
            if actual.representative != Some(*expected) {
                report.difference(
                    format!("{label}[{ordinal}].representative"),
                    actual.representative,
                    expected,
                );
            }
        }
    }
    for handle in captured.keys() {
        if !expected.contains_key(handle) {
            report.difference(
                format!("theatre {handle} controller areas"),
                "present",
                "no saved volunteer theatre",
            );
        }
    }
    report.compared("TheatreAreas", compared);
    report.limitation(
        "CTheatre serializer EED430 emits one representative province per controller area through area+0x28, never the area's whole +0x30 membership; the captured province list has no save representation",
        "TheatreAreas.provinces",
    );
    Ok(())
}

fn record_radar(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::RadarCoverage>(frame)?;
    report.compared("RadarCoverage", 0);
    report.limitation(
        "no inspected country serializer path writes the CCountry+0x1120 radar coverage vectors, so this table has no save comparison; the recorded rows decode but are not certified against a save",
        format!("RadarCoverage: {} decoded rows, zero save comparisons", rows.len()),
    );
    Ok(())
}

fn record_registrations(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::AirBaseRegistrations>(frame)?;
    report.compared("AirBaseRegistrations", 0);
    report.limitation(
        "CAirBase serializer C57070 does not traverse the +0x30 country containers, and a wing pool's assigned base is not the registration, so this table has no save comparison",
        format!(
            "AirBaseRegistrations: {} decoded rows, zero save comparisons",
            rows.len()
        ),
    );
    Ok(())
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::save::{self, parse};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            AirBaseRegistrationRow, RadarCoverageRow, StationedUnitRow, TableId, TableRows,
            TheatreAreaRow, UnitIntelRow, VolunteerHostRow,
        },
    };

    const DIVISION: u64 = (9_u64 << 32) | 4919;
    const THEATRE: u64 = (5_u64 << 32) | 67;

    fn frame(rows: impl IntoIterator<Item = TableRows>) -> DecodedFrame {
        let mut tables =
            std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
        for rows in rows {
            let index = rows.table().index();
            tables[index] = TableView::Complete(rows);
        }
        DecodedFrame {
            branch: 1,
            hour: 7,
            tables,
            metrics: Metrics::default(),
        }
    }

    fn saved() -> save::Save {
        parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={
 GER={
  received_air_volunteer_permission={ tag=SOV count=3 }
  received_air_volunteer_permission={ tag=SOV count=-1 }
  units={ division={ id={id=9 type=4919} logical_country=GER strength=0 organisation=0
    country_intel={ 2 0 4 -1 0 -1 } } }
  theatres={ theatre={ id={id=5 type=67} volunteers_theatre=SOV
    unit={id=9 type=4919} area={ 21 22 } } }
 }
 SOV={}
}
states={} provinces={}"#,
        )
        .unwrap()
    }

    fn links() -> (CountryRow, CountryRow) {
        (
            CountryRow {
                id: 12,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 34,
                canonical_slot: 2,
                tag: b"SOV".to_vec(),
                ..Default::default()
            },
        )
    }

    fn complete_frame() -> DecodedFrame {
        frame([
            TableRows::UnitIntel(vec![
                UnitIntelRow {
                    unit: DIVISION,
                    ordinal: 0,
                    viewer: 2,
                    level: 4,
                },
                UnitIntelRow {
                    unit: DIVISION,
                    ordinal: 1,
                    viewer: u32::MAX,
                    level: -1,
                },
            ]),
            TableRows::VolunteerHosts(vec![
                VolunteerHostRow {
                    country: 12,
                    position: 0,
                    host: 34,
                    deployed_divisions: 3,
                },
                VolunteerHostRow {
                    country: 12,
                    position: 1,
                    host: 34,
                    deployed_divisions: -1,
                },
            ]),
            TableRows::StationedUnits(vec![StationedUnitRow {
                theatre: THEATRE,
                ordinal: 0,
                unit: DIVISION,
            }]),
            TableRows::TheatreAreas(vec![
                TheatreAreaRow {
                    theatre: THEATRE,
                    ordinal: 0,
                    representative: Some(21),
                    provinces: vec![21, 99],
                },
                TheatreAreaRow {
                    theatre: THEATRE,
                    ordinal: 1,
                    representative: Some(22),
                    provinces: Vec::new(),
                },
            ]),
            TableRows::RadarCoverage(vec![RadarCoverageRow {
                country: 12,
                ordinal: 0,
                state: Some(4),
                provinces: vec![7],
            }]),
            TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
                base: 65,
                slot: 1,
                country: 12,
                wings: vec![69],
            }]),
        ])
    }

    #[test]
    fn every_saved_family_matches_and_the_unsaved_ones_are_named_limitations() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut report = Report::default();
        compare(&complete_frame(), &saved(), &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["UnitIntel"], 2);
        assert_eq!(report.compared_tables["VolunteerHosts"], 2);
        assert_eq!(report.compared_tables["StationedUnits"], 1);
        assert_eq!(report.compared_tables["TheatreAreas"], 2);
        assert_eq!(report.compared_tables["RadarCoverage"], 0);
        assert_eq!(report.compared_tables["AirBaseRegistrations"], 0);
        for expected in [
            "decay accumulator",
            "representative province per controller area",
            "radar coverage vectors",
            "does not traverse the +0x30 country containers",
        ] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(expected)),
                "{expected}"
            );
        }
    }

    #[test]
    fn changed_levels_hosts_units_and_representatives_are_differences() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let save = saved();
        for mutate in [
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::UnitIntel(rows)) =
                    &mut frame.tables[TableId::UnitIntel.index()]
                else {
                    unreachable!()
                };
                rows[1].level = 0;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::VolunteerHosts(rows)) =
                    &mut frame.tables[TableId::VolunteerHosts.index()]
                else {
                    unreachable!()
                };
                rows[0].deployed_divisions = 4;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::StationedUnits(rows)) =
                    &mut frame.tables[TableId::StationedUnits.index()]
                else {
                    unreachable!()
                };
                rows[0].unit = 1;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::TheatreAreas(rows)) =
                    &mut frame.tables[TableId::TheatreAreas.index()]
                else {
                    unreachable!()
                };
                rows[1].representative = None;
            },
        ] {
            let mut frame = complete_frame();
            mutate(&mut frame);
            let mut report = Report::default();
            // A difference is recorded, not raised: only a comparison that
            // cannot run at all fails its group.
            compare(&frame, &save, &links, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        }
    }

    #[test]
    fn missing_rows_extra_parents_and_unavailable_tables_never_pass_silently() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let save = saved();
        let mut frame = complete_frame();
        frame.tables[TableId::UnitIntel.index()] =
            TableView::Complete(TableRows::UnitIntel(vec![]));
        let mut report = Report::default();
        compare(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.iter().any(|line| line.contains("count")));

        let mut frame = complete_frame();
        let TableView::Complete(TableRows::StationedUnits(rows)) =
            &mut frame.tables[TableId::StationedUnits.index()]
        else {
            unreachable!()
        };
        rows[0].theatre = (6_u64 << 32) | 67;
        let mut report = Report::default();
        compare(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);

        let mut frame = complete_frame();
        frame.tables[TableId::RadarCoverage.index()] = TableView::Unavailable {
            reason: "guarded copy failed".into(),
        };
        let mut report = Report::default();
        assert!(compare(&frame, &save, &links, &mut report).is_err());
        assert!(report.failed_comparisons.contains_key("fog RadarCoverage"));
    }

    #[test]
    fn a_gap_in_the_native_ordinals_refuses_the_comparison() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut frame = complete_frame();
        let TableView::Complete(TableRows::UnitIntel(rows)) =
            &mut frame.tables[TableId::UnitIntel.index()]
        else {
            unreachable!()
        };
        rows[1].ordinal = 3;
        let mut report = Report::default();
        assert!(compare(&frame, &saved(), &links, &mut report).is_err());
        assert!(report.failed_comparisons["fog UnitIntel"].contains("native ordinal"));
    }

    #[test]
    fn a_save_without_a_theatre_roster_refuses_both_stationed_comparisons() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut save = saved();
        save.naval_history.theatre_countries = None;
        let mut report = Report::default();
        assert!(compare(&complete_frame(), &save, &links, &mut report).is_err());
        assert!(
            report.failed_comparisons["fog StationedUnits"].contains("no theatre roster")
                && report.failed_comparisons["fog TheatreAreas"].contains("no theatre roster")
        );
    }
}
