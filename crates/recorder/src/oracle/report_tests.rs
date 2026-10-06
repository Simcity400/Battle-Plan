use super::*;

#[test]
fn membership_exclusions_are_counted_separately_from_failed_groups() {
    let membership = state_history_tests::membership(
        "state={id=1 provinces={1} history={free_operative={operative=a}}}",
        1,
    );
    let mut report = Report::default();
    report.membership_skip("Provinces", 1, &membership, &BTreeSet::from([1]));
    for summary in [
        report.coverage_line(),
        report.outcome().unwrap_err().to_string(),
    ] {
        assert!(
            summary.contains("0 failed or skipped comparison groups"),
            "{summary}"
        );
        assert!(
            summary.contains("1 groups with membership exclusions"),
            "{summary}"
        );
        assert!(summary.contains("incomplete"), "{summary}");
    }
}

/// At a non-full hour, a daily group not read at that hour is carried,
/// not compared: its comparison does not run, it is listed with its last
/// read hour, and the oracle can pass. Groups read at the hour, hourly
/// groups, groups naming no table and a full hour are compared as before.
#[test]
fn carried_daily_groups_are_listed_not_compared_at_a_non_full_hour() {
    let (slot, hour) = match replay::cadence::cadence(TableId::CountryFuel) {
        replay::cadence::Cadence::Daily(slot) => (slot as Hour, 24 * 1000 + 5_i32),
        other => panic!("CountryFuel is {other:?}"),
    };
    assert_ne!(
        hour.rem_euclid(24),
        slot,
        "the compared hour is off the slot"
    );
    let last_read = 24 * 1000 + slot - if slot < 5 { 0 } else { 24 };
    let mut report = Report {
        hour,
        read_hours: Some(BTreeMap::from([(TableId::CountryFuel, last_read)])),
        ..Report::default()
    };
    let differ = |report: &mut Report| {
        report.difference("field", 1, 2);
        Ok(())
    };
    // Carried: not run, listed, and the report passes.
    assert!(report.attempt("economy CountryFuel", differ));
    assert!(report.differences.is_empty());
    assert_eq!(
        report.carried,
        BTreeMap::from([("economy CountryFuel".to_owned(), Some(last_read))])
    );
    assert!(report.outcome().is_ok());
    let line = report.coverage_line();
    assert!(
        line.starts_with("Comparison status for recorded-format coverage: complete; 0 table availability issues; 0 failed or skipped comparison groups"),
        "{line}"
    );
    assert!(
        line.contains("1 comparison groups or sliced scopes carried, not compared"),
        "{line}"
    );
    // Multi-table daily groups are carried as one.
    assert!(report.attempt("Deployment", differ));
    assert_eq!(report.carried.get("Deployment"), Some(&None));
    // A group naming no table, or an hourly table, is compared.
    report.attempt("provinces", differ);
    report.attempt_table(TableId::Countries, differ);
    assert_eq!(report.differences.len(), 2);
    // A daily table read at this hour (its slot's walk was measured) is
    // compared; so is every table on a full hour.
    report
        .observed_refresh
        .push(replay::cadence::cadence(TableId::CountryFuel));
    report.attempt("economy CountryFuel", differ);
    assert_eq!(report.differences.len(), 3);
    report.observed_refresh.clear();
    report.forced_full = true;
    report.attempt("economy CountryFuel", differ);
    assert_eq!(report.differences.len(), 4);
    // CountryIntel counts as read only when its walk was measured: a
    // deferral on its slot hour is carried, a retry off its slot is
    // compared.
    report.forced_full = false;
    let intel_slot = match replay::cadence::cadence(TableId::CountryIntel) {
        replay::cadence::Cadence::Daily(slot) => slot as Hour,
        other => panic!("CountryIntel is {other:?}"),
    };
    report.hour = 24 * 1000 + intel_slot;
    report.observed_refresh.clear();
    assert!(report.attempt("CountryIntel", differ));
    assert!(
        report.carried.contains_key("CountryIntel"),
        "deferred on its slot"
    );
    report.carried.clear();
    report.hour += 1;
    report.intel_walked = true;
    let before = report.differences.len();
    report.attempt("CountryIntel", differ);
    assert_eq!(report.differences.len(), before + 1, "retried off its slot");
    assert!(report.carried.is_empty());
    // Without read hours (tests and callers that do not scan the branch)
    // nothing is carried.
    let mut unscoped = Report {
        hour,
        ..Report::default()
    };
    unscoped.attempt("economy CountryFuel", differ);
    assert_eq!(unscoped.differences.len(), 1);
    assert!(unscoped.carried.is_empty());
}

/// A daily table is read at its slot hour although the shared read was
/// charged to an hourly anchor (part Q: every Daily(17) table timed zero
/// at slot 17, the economy-rest copy charged to ProvinceSupply); off the
/// slot only a timed table of the slot proves a read; CountryIntel only by
/// its own walk.
#[test]
fn a_daily_read_is_dated_by_its_slot_not_by_its_charged_timing() {
    let slot = match replay::cadence::cadence(TableId::CountryFuel) {
        replay::cadence::Cadence::Daily(slot) => slot as Hour,
        other => panic!("CountryFuel is {other:?}"),
    };
    let frame = |hour, timed: &[TableId]| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        for table in timed {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    };
    let read = |frame: &DecodedFrame, table| daily_read(frame.hour, &frame.metrics, table);
    let hour = 24 * 1000 + slot;
    assert!(read(
        &frame(hour, &[TableId::ProvinceSupply]),
        TableId::CountryFuel
    ));
    assert!(read(&frame(hour, &[]), TableId::CountryFuelConsumers));
    assert!(!read(
        &frame(hour + 1, &[TableId::ProvinceSupply]),
        TableId::CountryFuel
    ));
    assert!(read(
        &frame(hour + 1, &[TableId::CountryFuelConsumers]),
        TableId::CountryFuel
    ));
    let lend_slot = match replay::cadence::cadence(TableId::LendLeaseEquipment) {
        replay::cadence::Cadence::Daily(slot) => 24 * 1000 + slot as Hour,
        other => panic!("LendLeaseEquipment is {other:?}"),
    };
    assert!(read(
        &frame(lend_slot, &[TableId::Divisions]),
        TableId::LendLeaseEquipment
    ));
    assert!(!read(
        &frame(lend_slot + 1, &[TableId::Divisions]),
        TableId::LendLeaseEquipment
    ));
    let intel = match replay::cadence::cadence(TableId::CountryIntel) {
        replay::cadence::Cadence::Daily(slot) => 24 * 1000 + slot as Hour,
        other => panic!("CountryIntel is {other:?}"),
    };
    assert!(!read(
        &frame(intel, &[TableId::LogisticsEquipment]),
        TableId::CountryIntel
    ));
    assert!(read(
        &frame(intel + 1, &[TableId::CountryIntel]),
        TableId::CountryIntel
    ));
    assert!(!read(
        &frame(intel + 1, &[TableId::CountryIntel]),
        TableId::LogisticsEquipment
    ));
}

/// LogisticsEquipment shares CountryIntel's slot. A deferred CountryIntel
/// walk leaves CountryIntel carried on the slot hour although LogisticsEquipment
/// was walked, and its retry at the next hour does not make the carried
/// logistics rows current; the scan of read hours agrees.
#[test]
fn country_intel_and_the_rest_of_its_slot_are_no_evidence_for_each_other() {
    let slot = match replay::cadence::cadence(TableId::CountryIntel) {
        replay::cadence::Cadence::Daily(slot) => slot as Hour,
        other => panic!("CountryIntel is {other:?}"),
    };
    assert_eq!(
        replay::cadence::cadence(TableId::LogisticsEquipment),
        replay::cadence::cadence(TableId::CountryIntel)
    );
    let differ = |report: &mut Report| {
        report.difference("field", 1, 2);
        Ok(())
    };
    let hour = 24 * 1000 + slot;
    let read_hours = BTreeMap::from([
        (TableId::CountryIntel, hour - 24),
        (TableId::LogisticsEquipment, hour),
    ]);
    for (hour, walked, intel, logistics) in [
        (hour, TableId::LogisticsEquipment, false, true),
        (hour + 1, TableId::CountryIntel, true, false),
    ] {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        frame.metrics.table_ns[walked.index()] = 1;
        let mut report = Report {
            read_hours: Some(read_hours.clone()),
            ..Report::default()
        };
        report.record_tables(&frame);
        assert!(!report.full_refresh(), "hour {hour}");
        report.attempt("CountryIntel", differ);
        report.attempt("economy LogisticsEquipment", differ);
        assert_eq!(
            (
                !report.carried.contains_key("CountryIntel"),
                !report.carried.contains_key("economy LogisticsEquipment"),
            ),
            (intel, logistics),
            "hour {hour}"
        );
    }
}

#[test]
fn every_difference_prints_under_its_own_comparison_group() {
    let mut report = Report::default();
    for group in ["alpha", "beta"] {
        report.attempt(group, |report| {
            for index in 0..80 {
                report.difference(format!("{group}.field{index}"), index, index + 1);
            }
            Ok(())
        });
    }
    assert_eq!(report.differences.len(), 160);
    assert_eq!(report.difference_owners.len(), 160);
    let lines = report.difference_lines();
    // One header, one count line per group, and every single difference.
    assert_eq!(lines.len(), 1 + 2 + 160);
    assert!(lines[0].contains("All 160 differences"));
    assert!(lines.contains(&"  alpha: 80 differences".to_owned()));
    assert!(lines.contains(&"  beta: 80 differences".to_owned()));
    for group in ["alpha", "beta"] {
        for index in 0..80 {
            let expected = format!(
                "    {group}.field{index}: replay {index}, save {}",
                index + 1
            );
            assert!(lines.contains(&expected), "{expected} was not printed");
        }
    }
    assert_eq!(
        report.difference_groups,
        BTreeMap::from([("alpha".to_owned(), 80), ("beta".to_owned(), 80)])
    );
}

#[test]
fn a_report_without_differences_prints_no_difference_section() {
    assert!(Report::default().difference_lines().is_empty());
}

#[test]
fn compatibility_field_limitations_aggregate_counts_without_duplicate_subjects() {
    use replay::rows::{Recorded, WeatherProvinceRow};

    let (mut frame, ..) = tests::fixture();
    let rows: Vec<_> = (0..13_413)
        .map(|id| WeatherProvinceRow {
            id,
            temperature: if id == 0 {
                Recorded::Value(0)
            } else {
                Recorded::NotRecorded
            },
            raw_mud: Recorded::NotRecorded,
            ..Default::default()
        })
        .collect();
    for truncated in [false, true] {
        *frame.tables[TableId::WeatherProvinces.index()] = if truncated {
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(rows.clone()),
                missing: 7,
            }
        } else {
            TableView::Complete(TableRows::WeatherProvinces(rows.clone()))
        };
        let mut report = Report::default();
        report.record_tables(&frame);
        for row in &rows {
            report.recorded(
                &row.temperature,
                "WeatherProvinces.temperature",
                row.id.to_string(),
            );
            report.recorded(
                &row.raw_mud,
                "WeatherProvinces.raw_mud",
                row.id.to_string(),
            );
        }
        assert_eq!(report.limitations.len(), 2);
        for (field, count) in [("temperature", 13_412), ("raw_mud", 13_413)] {
            assert_eq!(
                report.limitations[&format!("WeatherProvinces.{field} not recorded")],
                [format!("WeatherProvinces.{field}: {count} rows not recorded")]
            );
        }
    }
}
