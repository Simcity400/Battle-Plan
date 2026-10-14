//! Session comparisons: the saved speed index and the world-threat sources.
//!
//! The world-threat writer saves only the ordered source vector.
//! The manager total and the per-country aggregate cache are rebuilt on load, so neither has a saved counterpart; both are
//! reported as printed limitations rather than skipped.

use std::{collections::BTreeMap, io};

use installed_content_save::Save;
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, complete, country_tag, invalid, tag_text};

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut ok = report.attempt("GameSpeed", |report| compare_speed(frame, saved, report));
    ok &= report.attempt("ThreatSources", |report| {
        compare_sources(frame, saved, links, report)
    });
    ok &= report.attempt("WorldTension", |report| {
        compare_tension(frame, saved, report)
    });
    ok &= report.attempt("ThreatCountryAggregates", |report| {
        compare_aggregates(frame, report)
    });
    if ok {
        Ok(())
    } else {
        Err(invalid("one or more session comparisons failed"))
    }
}

fn compare_speed(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let [clock] = complete::<tables::Clock>(frame)? else {
        return Err(invalid("clock table must contain one clock"));
    };
    match saved.session.speed {
        Some(speed) => {
            if clock.speed != speed {
                report.difference("clock.speed", clock.speed, speed);
            }
            report.compared("GameSpeed", 1);
        }
        None => report.difference("clock.speed", clock.speed, "missing serialized speed"),
    }
    report.limitation(
        "speed is a live setting the player can change between the recorded callback and the save; setter runs on the main thread outside the hourly tick and clamps to 0..=4",
        "Clock.speed",
    );
    Ok(())
}

fn compare_sources(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let recorded = complete::<tables::ThreatSources>(frame)?;
    for (position, row) in recorded.iter().enumerate() {
        if row.ordinal as usize != position {
            return Err(invalid(
                "captured threat sources are not a dense ordinal sequence",
            ));
        }
    }
    let Some(expected) = &saved.session.threat_sources else {
        report.difference(
            "threat.sources",
            format!("{} decoded native rows", recorded.len()),
            "missing serialized threat block",
        );
        return Ok(());
    };
    if recorded.len() != expected.len() {
        report.difference("threat.source_count", recorded.len(), expected.len());
    }
    let mut compared = 0;
    for (actual, expected) in recorded.iter().zip(expected) {
        let label = format!("threat source {}", expected.ordinal);
        if !expected.extra_keys.is_empty() {
            report.difference(
                format!("{label}.unmodelled_keys"),
                "none",
                expected
                    .extra_keys
                    .iter()
                    .map(|key| String::from_utf8_lossy(key).into_owned())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        compared += 1;
        for (field, captured, saved) in [
            ("threat", actual.current_raw, expected.threat.raw),
            ("final_threat", actual.final_raw, expected.final_threat.raw),
            ("daily", actual.daily_raw, expected.daily.raw),
        ] {
            if captured != saved {
                report.difference(format!("{label}.{field}"), captured, saved);
            }
        }
        // The game writes either tag only for a signed-positive reference.
        for (field, reference, expected) in [
            ("tag", actual.country, expected.tag.as_deref()),
            ("target", actual.second_country, expected.target.as_deref()),
        ] {
            let actual = if reference as i32 > 0 {
                country_tag(reference, links)?
            } else {
                None
            };
            if actual != expected {
                report.difference(
                    format!("{label}.{field}"),
                    tag_text(actual),
                    tag_text(expected),
                );
            }
        }
        let date = expected.date.engine_hour()?;
        if actual.date != date {
            report.difference(format!("{label}.date"), actual.date, date);
        }
        match &expected.label {
            Some(text) if &actual.reason != text => report.difference(
                format!("{label}.label"),
                String::from_utf8_lossy(&actual.reason).into_owned(),
                String::from_utf8_lossy(text).into_owned(),
            ),
            Some(_) => {}
            None => report.difference(
                format!("{label}.label"),
                String::from_utf8_lossy(&actual.reason).into_owned(),
                "missing serialized label",
            ),
        }
    }
    report.compared("ThreatSources", compared);
    report.limitation(
        "serialized tag and target compare resolved country identity and cannot certify equivalent raw country-link aliases; the writer emits both tags and omits a reference that is not positive",
        "ThreatSources.country, ThreatSources.second_country",
    );
    Ok(())
}

fn compare_tension(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let [tension] = complete::<tables::WorldTension>(frame)? else {
        return Err(invalid("world tension table must contain one row"));
    };
    if let Some(expected) = &saved.session.threat_sources {
        let sources = i32::try_from(expected.len())
            .map_err(|_| invalid("saved threat source count exceeds i32"))?;
        if tension.source_count != sources {
            report.difference("world_tension.source_count", tension.source_count, sources);
        }
        report.compared("WorldTension", 1);
    }
    report.limitation(
        "the world-threat writer saves only its source vector: the resident total and aggregate count are rebuilt on load by the game and have no saved counterpart. The total is clamp(sum of source contributions, 0, 10000000)/100 as of the last update, not a serialized value",
        "WorldTension.total_raw, WorldTension.country_aggregate_count",
    );
    Ok(())
}

fn compare_aggregates(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let recorded = complete::<tables::ThreatCountryAggregates>(frame)?;
    for (slot, row) in recorded.iter().enumerate() {
        if row.country_slot as usize != slot {
            return Err(invalid(
                "captured threat aggregates are not a dense slot sequence",
            ));
        }
    }
    report.limitation(
        "the per-country threat cache is rebuilt by the game and is never serialized; its slots are compared against no save value",
        format!(
            "ThreatCountryAggregates: {} decoded rows, zero save comparisons",
            recorded.len()
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            ClockRow, TableId, TableRows, ThreatCountryAggregateRow, ThreatSourceRow,
            WorldTensionRow,
        },
    };

    fn roster() -> Vec<CountryRow> {
        vec![CountryRow {
            id: 7,
            canonical_slot: 7,
            tag: b"ENG".to_vec(),
            ..Default::default()
        }]
    }

    const SAVE: &[u8] = br#"HOI4txt
date=1936.1.2.13 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
speed=3
countries={} states={} provinces={}
threat={
    threat={ threat=-1.71336 final_threat=90071992547.40993 daily=0
             tag="ENG" date="1936.1.1.1" label="Peace" }
    threat={ threat=0 final_threat=0 daily=-0.00001 date="1936.1.1.1" label="" }
}"#;

    fn frame(hour: i32) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "outside session comparison".into(),
            })
            .map(Into::into),
        };
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour,
                speed: 3,
                ..Default::default()
            }]));
        *frame.tables[TableId::WorldTension.index()] =
            TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow {
                total_raw: -1_256,
                source_count: 2,
                country_aggregate_count: 3,
            }]));
        *frame.tables[TableId::ThreatSources.index()] =
            TableView::Complete(TableRows::ThreatSources(vec![
                ThreatSourceRow {
                    ordinal: 0,
                    country: 7,
                    second_country: 0,
                    current_raw: -171_336,
                    final_raw: 9_007_199_254_740_993,
                    daily_raw: 0,
                    date: 1936 * 365 * 24 + 43_800_000,
                    reason: b"Peace".to_vec(),
                },
                ThreatSourceRow {
                    ordinal: 1,
                    daily_raw: -1,
                    date: 1936 * 365 * 24 + 43_800_000,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::ThreatCountryAggregates.index()] =
            TableView::Complete(TableRows::ThreatCountryAggregates(vec![
                ThreatCountryAggregateRow {
                    country_slot: 0,
                    total_raw: i64::MIN,
                },
                ThreatCountryAggregateRow {
                    country_slot: 1,
                    total_raw: 0,
                },
            ]));
        frame
    }

    #[test]
    fn saved_speed_and_ordered_sources_compare_exactly_with_named_limitations() {
        let saved = installed_content_save::parse(SAVE).unwrap();
        let frame = frame(saved.date.engine_hour().unwrap());
        let countries = roster();
        let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
        let mut report = Report::default();
        compare(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["ThreatSources"], 2);
        assert_eq!(report.compared_tables["GameSpeed"], 1);
        assert_eq!(report.compared_tables["WorldTension"], 1);
        // The aggregate cache is never serialized and reports no comparisons.
        assert!(
            !report
                .compared_tables
                .contains_key("ThreatCountryAggregates")
        );
        assert_eq!(report.limitations.len(), 4);
    }

    #[test]
    fn threat_targets_compare_the_second_reference_and_its_omission_gate() {
        let source = String::from_utf8(SAVE.to_vec())
            .unwrap()
            .replace("tag=\"ENG\"", "tag=\"ENG\" target=\"ENG\"");
        let saved = installed_content_save::parse(source.as_bytes()).unwrap();
        let mut frame = frame(saved.date.engine_hour().unwrap());
        let countries = roster();
        let links = countries.iter().map(|row| (row.id, row)).collect();
        let TableView::Complete(TableRows::ThreatSources(rows)) =
            &mut *frame.tables[TableId::ThreatSources.index()]
        else {
            unreachable!()
        };
        rows[0].second_country = 7;
        let mut report = Report::default();
        compare_sources(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::ThreatSources(rows)) =
            &mut *frame.tables[TableId::ThreatSources.index()]
        else {
            unreachable!()
        };
        rows[0].second_country = 0;
        rows[1].second_country = u32::MAX;
        let mut report = Report::default();
        compare_sources(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains(".target"));
    }

    #[test]
    fn every_changed_word_speed_and_missing_label_is_a_difference() {
        let saved = installed_content_save::parse(SAVE).unwrap();
        let hour = saved.date.engine_hour().unwrap();
        let countries = roster();
        let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
        let mut frame = frame(hour);
        let TableView::Complete(TableRows::ThreatSources(rows)) =
            &mut *frame.tables[TableId::ThreatSources.index()]
        else {
            unreachable!()
        };
        rows[0].current_raw += 1;
        rows[0].final_raw -= 1;
        rows[1].daily_raw = 0;
        rows[0].reason = b"War".to_vec();
        rows[0].date += 1;
        let TableView::Complete(TableRows::Clock(rows)) =
            &mut *frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].speed = 4;
        let mut report = Report::default();
        compare(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 6, "{:?}", report.differences);
    }

    #[test]
    fn absent_saves_broken_ordinals_and_unavailable_tables_never_pass_silently() {
        let saved = installed_content_save::parse(
            br#"HOI4txt
date=1936.1.2.13 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
countries={} states={} provinces={}"#,
        )
        .unwrap();
        let hour = saved.date.engine_hour().unwrap();
        let countries = roster();
        let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
        let mut report = Report::default();
        compare(&frame(hour), &saved, &links, &mut report).unwrap();
        // Missing speed and a missing threat block are both differences.
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        let mut broken = frame(hour);
        let TableView::Complete(TableRows::ThreatSources(rows)) =
            &mut *broken.tables[TableId::ThreatSources.index()]
        else {
            unreachable!()
        };
        rows[1].ordinal = 5;
        assert!(compare(&broken, &saved, &links, &mut Report::default()).is_err());

        let mut unavailable = frame(hour);
        *unavailable.tables[TableId::WorldTension.index()] = TableView::Unavailable {
            reason: "world threat vtable mismatch".into(),
        };
        assert!(compare(&unavailable, &saved, &links, &mut Report::default()).is_err());
    }
}
