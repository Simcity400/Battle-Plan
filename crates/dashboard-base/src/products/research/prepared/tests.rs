use super::*;
use crate::prepared::series::tests::Fixture;
use crate::products::research::test_fixtures::sliced_research_frame;
use replay::frame::TableView;
use replay::rows::{
    CountryLinkRow, CountryRow, ResearchInputRow, SliceReadRow, TableId, TableRows,
};

#[test]
fn research_deadlines_expire_exactly_across_carried_spans_gaps_commits_resume_and_append() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut reference = Scan::default();
    let flag = AtomicBool::new(false);
    let first = sliced_research_frame(100, 100, &[Some(1), None]);
    let format = crate::test_fixtures::replay(vec![first.clone()]).format;
    let mut tracker = FrameChangeTracker::new();
    let mut frame = first;
    for hour in 100..=182 {
        if hour == 140 {
            continue;
        }
        if [129, 155].contains(&hour) {
            frame = sliced_research_frame(hour, hour, &[Some(2), None]);
        }
        frame.hour = hour;
        reference.observe_selected(&frame, None).unwrap();
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        if [124, 179].contains(&hour) {
            assert!(prepared.scan.countries["GER"].carry_until.is_none());
            assert_eq!(
                prepared.scan.countries["GER"]
                    .history
                    .coverage
                    .last()
                    .unwrap()
                    .last_hour,
                hour - 1
            );
        }
        if [110, 130, 182].contains(&hour) {
            let mut writer = if hour == 182 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            compare(&fixture, &reference, "GER");
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    let key = prepared.keys["GER"];
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    assert!(reader.value_at(key, 123).unwrap().is_some());
    assert!(reader.value_at(key, 124).unwrap().is_none());
    assert!(reader.value_at(key, 139).unwrap().is_some());
    assert!(reader.value_at(key, 140).unwrap().is_none());
    assert!(reader.value_at(key, 141).unwrap().is_none());
    assert!(reader.value_at(key, 178).unwrap().is_some());
    assert!(reader.value_at(key, 179).unwrap().is_none());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn research_work_is_flat_over_unchanged_hours_and_retired_holder_history() {
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let first = sliced_research_frame(100, 100, &[Some(1)]);
    let format = crate::test_fixtures::replay(vec![first.clone()]).format;
    observer_tests::assert_flat(
        observer_tests::scaled_work(100, |hours| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(&first, &tracker.observe(&first, format))?;
            observer_tests::unchanged_frames(
                prepared.scan.work,
                &first,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_changed(frame, changes)?;
                    Ok(prepared.scan.work)
                },
            )
        })
        .unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |history| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            for offset in 0..history {
                let hour = 100 + offset as i32;
                let mut frame = sliced_research_frame(hour, hour, &[Some(1)]);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = format!("retired research holder {offset}").into_bytes();
                }
                prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
            }
            let hour = 100 + history as i32;
            let frame = sliced_research_frame(hour, hour, &[Some(2)]);
            let before = prepared.scan.work;
            prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
            Ok(Work {
                row_visits: prepared.scan.work.row_visits - before.row_visits,
                history_visits: prepared.scan.work.history_visits - before.history_visits,
                ..Default::default()
            })
        })
        .unwrap(),
    );
}

fn compare(fixture: &Fixture, expected: &Scan, tag: &str) {
    let (mut actual, unjoined) = history(
        &fixture.store,
        &fixture.key,
        1,
        tag,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let mut reference = expected.countries[tag].history.clone();
    reference.slots.sort_by_key(|slot| slot.index);
    actual.slots.sort_by_key(|slot| slot.index);
    assert_eq!(actual, reference);
    assert_eq!(unjoined, expected.countries[tag].unjoined_reads);
}

#[test]
fn research_events_and_runs_match_after_two_commits_resume_append_gaps_and_holder_change() {
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut reference = Scan::default();
    let flag = AtomicBool::new(false);
    for (first, last) in [(100, 123), (124, 147)] {
        for hour in first..=last {
            let assignment = if hour < 124 { Some(1) } else { Some(2) };
            let frame = sliced_research_frame(hour, first, &[assignment, None]);
            reference.observe_selected(&frame, None).unwrap();
            prepared.observe(&frame).unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &reference, "GER");
        let state = values_query(
            "series-state",
            serde_json::json!({ "branchId": "1", "reads": [{ "tag": "GER", "hour": last }] }),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            state["states"][0]["slots"][0]["state"]["token"],
            if first == 100 { "tech_1" } else { "tech_2" }
        );
        assert_eq!(state["states"][0]["slots"][0]["readHour"], first);
        let summary = values_query("series-summary", serde_json::json!({ "branchId": "1", "tags": ["GER"], "from": first, "to": last, "width": 100 }), &fixture.store, &fixture.key, &flag).unwrap();
        assert_eq!(summary["series"].as_array().unwrap().len(), 2);
        let navigation = values_query("series-next-change", serde_json::json!({ "branchId": "1", "tags": ["GER"], "key": "Home", "firstHour": first, "lastHour": last }), &fixture.store, &fixture.key, &flag).unwrap();
        assert_eq!(navigation["hour"].as_f64(), Some(f64::from(first)));
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut writer = fixture.append_writer();
    for hour in 150..=176 {
        let read = if hour < 174 { 150 } else { 174 };
        let mut frame = sliced_research_frame(hour, read, &[Some(3), None]);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = 9002;
        }
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 9002,
                    canonical_slot: 7,
                },
            ]));
        reference.observe_selected(&frame, None).unwrap();
        prepared.observe(&frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &reference, "GER");
    assert_eq!(reference.countries["GER"].history.coverage.len(), 2);
}

#[test]
fn research_open_replies_keep_first_seen_key_order_and_missing_countries_across_commits() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut reference = Scan::default();
    let flag = AtomicBool::new(false);
    let mut first = sliced_research_frame(100, 100, &[Some(1)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"ZZZ".to_vec();
    }
    prepared.observe(&first).unwrap();
    reference.observe_selected(&first, None).unwrap();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &reference, "ZZZ");

    let mut second = sliced_research_frame(101, 101, &[Some(2)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"ZZZ".to_vec();
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"AAA".to_vec(),
            ..Default::default()
        });
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *second.tables[TableId::CountryLinks.index()]
    {
        rows.extend([
            CountryLinkRow {
                id: 12,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9002,
                canonical_slot: 8,
            },
        ]);
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::ResearchInputs(rows)) =
        &mut *second.tables[TableId::ResearchInputs.index()]
    {
        let mut input = rows[0].clone();
        input.country = 12;
        rows.push(input);
        rows.sort_by_key(|row| (row.country, row.index));
    }
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *second.tables[TableId::SliceReads.index()]
    {
        rows.extend(
            [
                TableId::ResearchInputs,
                TableId::ResearchAppliedBonuses,
                TableId::ResearchModifiers,
                TableId::ResearchTechnologies,
            ]
            .map(|table| SliceReadRow {
                table: table.index() as u32,
                owner: 12,
                sample_hour: 101,
            }),
        );
        rows.sort_by_key(|row| (row.table, row.owner));
    }
    prepared.observe(&second).unwrap();
    reference.observe_selected(&second, None).unwrap();
    assert_eq!(prepared.keys["ZZZ"], 0);
    assert!(
        prepared.keys["AAA"] > 1,
        "slot IDs leave gaps between country keys"
    );
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &reference, "ZZZ");
    compare(&fixture, &reference, "AAA");
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, &open_section(1))
            .unwrap(),
        Some(2)
    );

    // A retained catalogue key cannot create a hole or discard its last reply.
    prepared.scan.countries.remove("ZZZ");
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &reference, "ZZZ");
    compare(&fixture, &reference, "AAA");
}

fn scale(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = crate::test_fixtures::scale_frame(1, hour, count);
    *frame.tables[TableId::ResearchInputs.index()] =
        TableView::Complete(TableRows::ResearchInputs(
            (0..count)
                .map(|index| ResearchInputRow {
                    country: 100_000 + index,
                    index: 0,
                    technology_handle: 1,
                    database_index: replay::rows::Recorded::Value(1),
                    technology_present: 1,
                    token: b"synthetic_project".to_vec(),
                    classifier: engine::ctechnology::CLASSIFIER_ACTIVE,
                    cost_raw: 10_000_000,
                    cost_scale_raw: 100_000,
                    progress_raw: i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchAppliedBonuses.index()] =
        TableView::Complete(TableRows::ResearchAppliedBonuses(Vec::new()));
    let reads = (0..count)
        .map(|index| SliceReadRow {
            table: TableId::ResearchInputs.index() as u32,
            owner: 100_000 + index,
            sample_hour: hour,
        })
        .collect();
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    frame
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn research_observes_five_thousand_countries_once_and_checkpoints_only_continuation() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut reference = Scan::default();
    let flag = AtomicBool::new(false);
    for hour in 0..2 {
        let frame = scale(hour, 5_000);
        reference.observe_selected(&frame, None).unwrap();
        prepared.observe(&frame).unwrap();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(prepared.keys.len(), 5_000);
    for index in [0, 2_499, 4_999] {
        compare(
            &fixture,
            &reference,
            std::str::from_utf8(&crate::test_fixtures::scale_tag(index)).unwrap(),
        );
    }
    let mut prepared = Prepared::default();
    let mut writer = fixture.writer();
    let mut before = 0;
    for hour in 0..2_000 {
        prepared.observe(&scale(hour, 1)).unwrap();
        if hour == 199 {
            before = serde_json::to_vec(&prepared).unwrap().len();
        }
    }
    let after = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!(
        "research one-country checkpoint: {before} bytes after 200 reads, {after} after 2000 reads"
    );
    assert!(after <= before + 128);
    prepared.flush(&mut writer, 2, &flag).unwrap();
    writer.commit().unwrap();
}
