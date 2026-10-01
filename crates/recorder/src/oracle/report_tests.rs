use super::*;

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
    report
        .observed_refresh
        .push(replay::cadence::cadence(TableId::CountryIntel));
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
