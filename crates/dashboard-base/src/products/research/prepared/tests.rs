use super::*;
use crate::prepared::series::tests::Fixture;
use crate::products::research::test_fixtures::sliced_research_frame;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::ResearchAppliedBonusRow, rows::ResearchInputRow,
    rows::ResearchModifierRow, rows::ResearchTechnologyRow, rows::SliceReadRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::frame::TableView;
use std::sync::Arc;

#[test]
fn cold_and_prefix_research_states_match_prepared_values_at_the_supplying_read() {
    use crate::{
        prepared::coverage,
        test_fixtures::{file_source, source_with_content},
    };
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (_catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let frames: Vec<_> = (100..=149)
        .map(|hour| {
            let read = 100 + (hour - 100) / 24 * 24;
            sliced_research_frame(hour, read, &[Some(1), None])
        })
        .collect();
    let (memory, context) = source_with_content(frames.clone(), &folder.0);
    let (source, root) =
        file_source(replay::container::Replay::open(memory.replay_ref().bytes().to_vec()).unwrap());
    let (store, key) = crate::prepared::files(&source, context.content.as_ref()).unwrap();
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({ "branchId": "1", "family": { "kind": "research" }, "reads": [{ "tag": "GER", "hour": 149 }] });
    let cold = state_query(&source, &context, &params, &store, &key, &flag).unwrap();
    assert!(!cold.is_null());
    assert_eq!(cold["states"][0]["slots"][0]["readHour"], 148);
    assert_eq!(
        cold["states"][0]["preparing"],
        serde_json::json!(["assignment", "breakdown", "stateHistory", "remainingDays"])
    );
    assert!(!cold["pending"].as_array().unwrap().is_empty());
    let mut outside = params.clone();
    outside["reads"][0]["hour"] = serde_json::json!(151);
    let unrecorded = state_query(&source, &context, &outside, &store, &key, &flag).unwrap();
    assert_eq!(
        unrecorded["states"][0]["slots"],
        serde_json::json!([]),
        "a carry cannot invent an unrecorded hour"
    );
    let mut prepared = Prepared::default();
    for frame in &frames[..21] {
        prepared.observe(frame).unwrap();
    }
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    coverage::publish_coverage(&mut writer, 1, 100, 120, false).unwrap();
    writer.commit().unwrap();
    let prefix = state_query(&source, &context, &params, &store, &key, &flag).unwrap();
    assert_eq!(cold["states"][0]["data"], prefix["states"][0]["data"]);
    let reads = crate::prepared::query::frames::reads(&source);
    for frame in &frames[21..] {
        prepared.observe(frame).unwrap();
    }
    let mut writer = store.extend(&key).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    coverage::publish_coverage(&mut writer, 1, 100, 149, true).unwrap();
    writer.commit().unwrap();
    let complete = state_query(&source, &context, &params, &store, &key, &flag).unwrap();
    let exact_values = |reply: &serde_json::Value| -> serde_json::Value {
        serde_json::Value::Array(reply["states"][0]["slots"].as_array().unwrap().iter().map(|slot| {
            let state = &slot["state"];
            serde_json::json!({ "index": slot["index"], "available": slot["available"], "readHour": slot["readHour"],
                "token": state["token"], "speed": state["speedMultiplierRaw"], "banked": state["bankedDaysRaw"],
                "applied": state["appliedDaysRaw"], "ahead": state["aheadReductionYearsRaw"],
                "progress": slot["progress"]["progressRaw"], "base": slot["progress"]["baseDaysRaw"] })
        }).collect())
    };
    assert_eq!(
        exact_values(&cold),
        exact_values(&complete),
        "every claimed exact value is byte-equal"
    );
    assert!(
        cold["states"][0]["data"]["history"]["slots"][0]["states"][0]["details"]["remainingDays"]
            .is_null()
    );
    assert_eq!(complete["states"][0]["preparing"], serde_json::json!([]));
    assert_eq!(complete["pending"], serde_json::json!([]));
    assert_eq!(
        crate::prepared::query::frames::reads(&source),
        reads,
        "prepared state needs no recorded read"
    );
    store.clear().unwrap();
    drop(source);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn prepared_research_dates_empty_and_removed_slots_without_inventing_unavailable_reads() {
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut reference = Scan::default();
    let flag = AtomicBool::new(false);
    let mut unavailable = sliced_research_frame(102, 101, &[None]);
    *unavailable.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "synthetic missed research inputs".into(),
    };
    let steps = [
        (
            sliced_research_frame(100, 100, &[Some(1), None]),
            100,
            [true, true],
        ),
        (sliced_research_frame(101, 101, &[None]), 101, [true, false]),
        (unavailable, 101, [false, false]),
        (
            sliced_research_frame(103, 103, &[None, None]),
            103,
            [true, true],
        ),
    ];
    for (step, (frame, supplying_hour, available)) in steps.into_iter().enumerate() {
        reference.observe_selected(&frame, None).unwrap();
        prepared.observe(&frame).unwrap();
        let mut writer = if step == 3 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &reference, "GER");
        let reply = values_query(
            "series-state",
            serde_json::json!({ "branchId": "1", "reads": [{ "tag": "GER", "hour": frame.hour }] }),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        let slots = reply["states"][0]["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 2);
        for (index, slot) in slots.iter().enumerate() {
            assert_eq!(slot["index"], index);
            assert_eq!(slot["available"], available[index]);
            assert_eq!(slot["readHour"], supplying_hour);
            if !available[index] {
                assert!(slot["state"].is_null());
            } else if step != 0 || index == 1 {
                assert!(slot["state"]["token"].is_null());
            }
            if step == 0 && index == 0 {
                assert!(slot["progress"].is_object());
            } else {
                assert!(
                    slot["progress"].is_null(),
                    "an empty read has no old progress"
                );
            }
        }
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
}

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
                    database_index: replay_data::rows::Recorded::Value(1),
                    technology_present: 1,
                    token: b"synthetic_project".to_vec(),
                    classifier: engine_research::ctechnology::CLASSIFIER_ACTIVE,
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

/// The first research owner's id; owner `index` holds canonical slot `index`.
const FIRST_OWNER: u32 = 100_000;

fn complete(frame: &mut DecodedFrame, table: TableId) -> &mut TableRows {
    match &mut *frame.tables[table.index()] {
        TableView::Complete(rows) => rows,
        _ => unreachable!("fixture research tables are complete"),
    }
}
fn countries(frame: &mut DecodedFrame) -> &mut Vec<CountryRow> {
    match complete(frame, TableId::Countries) {
        TableRows::Countries(rows) => rows,
        _ => unreachable!(),
    }
}
fn links(frame: &mut DecodedFrame) -> &mut Vec<CountryLinkRow> {
    match complete(frame, TableId::CountryLinks) {
        TableRows::CountryLinks(rows) => rows,
        _ => unreachable!(),
    }
}
fn inputs(frame: &mut DecodedFrame) -> &mut Vec<ResearchInputRow> {
    match complete(frame, TableId::ResearchInputs) {
        TableRows::ResearchInputs(rows) => rows,
        _ => unreachable!(),
    }
}
fn technologies(frame: &mut DecodedFrame) -> &mut Vec<ResearchTechnologyRow> {
    match complete(frame, TableId::ResearchTechnologies) {
        TableRows::ResearchTechnologies(rows) => rows,
        _ => unreachable!(),
    }
}
fn modifiers(frame: &mut DecodedFrame) -> &mut Vec<ResearchModifierRow> {
    match complete(frame, TableId::ResearchModifiers) {
        TableRows::ResearchModifiers(rows) => rows,
        _ => unreachable!(),
    }
}
fn bonuses(frame: &mut DecodedFrame) -> &mut Vec<ResearchAppliedBonusRow> {
    match complete(frame, TableId::ResearchAppliedBonuses) {
        TableRows::ResearchAppliedBonuses(rows) => rows,
        _ => unreachable!(),
    }
}
fn slice_reads(frame: &mut DecodedFrame) -> &mut Vec<SliceReadRow> {
    match complete(frame, TableId::SliceReads) {
        TableRows::SliceReads(rows) => rows,
        _ => unreachable!(),
    }
}

/// `count` countries researching one technology each, all read at `hour`;
/// the first holds an applied bonus.
fn research_world(count: u32, hour: i32) -> DecodedFrame {
    let owner = |index: u32| FIRST_OWNER + index;
    let mut frame = crate::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..count)
            .map(|index| CountryRow {
                id: owner(index),
                canonical_slot: index,
                tag: crate::test_fixtures::scale_tag(index),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..count)
            .map(|index| CountryLinkRow {
                id: owner(index),
                canonical_slot: index,
            })
            .collect(),
    ));
    *frame.tables[TableId::ResearchInputs.index()] =
        TableView::Complete(TableRows::ResearchInputs(
            (0..count)
                .map(|index| ResearchInputRow {
                    country: owner(index),
                    index: 0,
                    technology_handle: 1,
                    database_index: replay_data::rows::Recorded::Value(1),
                    technology_present: 1,
                    token: b"synthetic_project".to_vec(),
                    classifier: engine_research::ctechnology::CLASSIFIER_ACTIVE,
                    cost_raw: 10_000_000,
                    cost_scale_raw: 100_000,
                    research_speed_raw: 100_000,
                    bonus_count: u32::from(index == 0),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(
            (0..count)
                .map(|index| ResearchTechnologyRow {
                    country: owner(index),
                    database_index: 1,
                    technology_handle: 1,
                    classifier: engine_research::ctechnology::CLASSIFIER_ACTIVE,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchModifiers.index()] =
        TableView::Complete(TableRows::ResearchModifiers(
            (0..count)
                .map(|index| ResearchModifierRow {
                    country: owner(index),
                    index: u32::MAX,
                    ordinal: 0,
                    kind: ResearchModifierRow::COUNTRY_SPEED,
                    raw: 10_000,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ResearchAppliedBonuses.index()] =
        TableView::Complete(TableRows::ResearchAppliedBonuses(vec![
            ResearchAppliedBonusRow {
                country: owner(0),
                index: 0,
                ordinal: 0,
                technology_handle: 1,
                id: 7,
                resolved: 1,
                name: b"synthetic_bonus".to_vec(),
                speed_bonus_raw: 50_000,
                ..Default::default()
            },
        ]));
    let mut reads: Vec<_> = [
        TableId::ResearchInputs,
        TableId::ResearchAppliedBonuses,
        TableId::ResearchModifiers,
        TableId::ResearchTechnologies,
    ]
    .into_iter()
    .flat_map(|table| {
        (0..count).map(move |index| SliceReadRow {
            table: table.index() as u32,
            owner: owner(index),
            sample_hour: hour,
        })
    })
    .collect();
    reads.sort_by_key(|row| (row.table, row.owner));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    frame
}

/// Owner `index`'s research read at `hour`: its progress advances.
fn read_at(frame: &mut DecodedFrame, index: u32, hour: i32) {
    let owner = FIRST_OWNER + index;
    for row in inputs(frame)
        .iter_mut()
        .filter(|row| row.country == owner && row.technology_present != 0)
    {
        row.progress_raw += 1_000;
    }
    for row in technologies(frame)
        .iter_mut()
        .filter(|row| row.country == owner)
    {
        row.progress_raw += 1_000;
    }
    for row in slice_reads(frame)
        .iter_mut()
        .filter(|row| row.owner == owner)
    {
        row.sample_hour = hour;
    }
}

/// The recording codec: each input hour is encoded and decoded as the
/// preparation pipeline reads it, with its recorded updates.
struct Recorded {
    encoder: replay_frame::frame::FrameEncoder,
    decoder: replay_frame::frame::FrameDecoder,
}
impl Recorded {
    fn new() -> Self {
        Self {
            encoder: replay_frame::frame::FrameEncoder::new(),
            decoder: replay_frame::frame::FrameDecoder::new(),
        }
    }
    fn next(
        &mut self,
        input: &DecodedFrame,
        keyframe: bool,
    ) -> (DecodedFrame, Arc<replay_frame::frame::Frame>) {
        let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (decoded, Arc::new(wire))
    }
}

fn assert_one_read_observes_one_country(count: u32) {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests::Work};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut recorded = Recorded::new();
    let mut tracker = FrameChangeTracker::new();
    let mut step = |prepared: &mut Prepared, frame: &DecodedFrame| {
        let before = (prepared.scan.work, prepared.scan.row_hours);
        let (decoded, wire) = recorded.next(frame, false);
        let changes = tracker.observe_updates(&decoded, wire, format);
        prepared.observe_changed(&decoded, &changes).unwrap();
        let work = Work {
            row_visits: prepared.scan.work.row_visits - before.0.row_visits,
            history_visits: prepared.scan.work.history_visits - before.0.history_visits,
            ..Default::default()
        };
        (work, prepared.scan.row_hours - before.1)
    };
    let mut prepared = Prepared::default();
    let mut frame = research_world(count, 100);
    let (first, _) = step(&mut prepared, &frame);
    assert_eq!(
        first.history_visits, count as usize,
        "the first hour reads every country"
    );

    // One country's read changes its technology's progress.
    frame.hour = 101;
    read_at(&mut frame, count / 2, 101);
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1, "the hour is observed from its changed rows");
    assert_eq!(work.history_visits, 1, "only the read country is observed");
    assert_eq!(
        work.row_visits, 2,
        "only its input row and its input read row are visited"
    );

    // Rows no read is joined by: another technology, a modifier and a
    // country field no holder is read from.
    frame.hour = 102;
    technologies(&mut frame)[(count / 3) as usize].progress_raw += 7;
    modifiers(&mut frame)[(count / 4) as usize].raw += 7;
    countries(&mut frame)[(count / 5) as usize].political_power_raw = Some(7);
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!(
        (work.row_visits, work.history_visits),
        (0, 0),
        "nothing a research read reads changed"
    );

    // A read beside a change to its own country's other fields.
    frame.hour = 103;
    read_at(&mut frame, count / 3, 103);
    countries(&mut frame)[(count / 3) as usize].political_power_raw = Some(9);
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!((work.row_visits, work.history_visits), (2, 1));
}

#[test]
fn research_hour_reading_one_country_observes_only_that_country() {
    assert_one_read_observes_one_country(200);
}

#[test]
fn research_hour_reading_one_of_5000_countries_observes_only_that_country() {
    assert_one_read_observes_one_country(5_000);
}

#[test]
fn research_holder_mask_names_both_holder_fields() {
    assert_eq!(super::super::HOLDER_FIELDS.count_ones(), 2);
}

/// Everything a publication holds for an observer's keys: each key's runs
/// and every event, in key order.
#[allow(clippy::type_complexity)]
fn published(
    fixture: &Fixture,
    observer: &Prepared,
) -> (
    Vec<(u64, Vec<series::Run>)>,
    Vec<(u64, i32, u64, serde_json::Value)>,
) {
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    let mut keys: Vec<u64> = observer.keys.values().copied().collect();
    keys.extend(
        observer
            .slots
            .values()
            .flat_map(BTreeMap::values)
            .flat_map(|keys| [keys.available, keys.state, keys.progress]),
    );
    keys.sort_unstable();
    let runs = keys
        .iter()
        .map(|&key| (key, reader.runs(key, i32::MIN, i32::MAX).unwrap()))
        .collect();
    let mut events = Vec::new();
    let mut cursor = 0;
    loop {
        let page = reader
            .events(&keys, i32::MIN, i32::MAX, cursor, 256)
            .unwrap();
        events.extend(
            page.rows
                .into_iter()
                .map(|row| (row.key, row.hour, row.ordinal, row.row)),
        );
        let Some(next) = page.cursor else {
            break;
        };
        assert!(next > cursor, "event pages advance");
        cursor = next;
    }
    (runs, events)
}

#[test]
fn research_rows_observation_matches_whole_tables_and_reference_across_keyframe_gap_and_resume() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let count = 6;
    let owner = |index: u32| FIRST_OWNER + index;
    let mut frame = research_world(count, 100);
    // The second country's slot has another owner, read on another day and
    // holding no rows: no read of that country proves its carry.
    links(&mut frame).push(CountryLinkRow {
        id: 200_001,
        canonical_slot: 1,
    });
    slice_reads(&mut frame).push(SliceReadRow {
        table: TableId::ResearchInputs.index() as u32,
        owner: 200_001,
        sample_hour: 90,
    });
    slice_reads(&mut frame).sort_by_key(|row| (row.table, row.owner));
    let fixtures = [Fixture::new(), Fixture::new()];
    // Observed from changed rows, and from the whole tables every hour.
    let mut observers = [
        Prepared::default(),
        Prepared {
            scan: Scan {
                whole_tables: true,
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut reference = Scan::default();
    let mut recorded = Recorded::new();
    let mut stashed: Option<Vec<ResearchInputRow>> = None;
    for hour in 100..=172 {
        if hour == 150 {
            // Never recorded: a gap.
            continue;
        }
        frame.hour = hour;
        // Each country is read once a day, four hours after the one before;
        // the sixth misses its read at 144.
        let due = (hour > 100)
            .then_some((hour - 100) % 24)
            .filter(|offset| offset % 4 == 0)
            .map(|offset| (offset / 4) as u32)
            .filter(|&index| index < count && (index, hour) != (5, 144));
        if let Some(index) = due {
            read_at(&mut frame, index, hour);
            if index == 1 {
                // Its states never merge across the hours its carry ends.
                inputs(&mut frame)
                    .iter_mut()
                    .find(|row| row.country == owner(1))
                    .unwrap()
                    .research_speed_raw += 1_000;
            }
        }
        match hour {
            // Rows no read joins: a modifier and a technology between reads.
            106 => {
                modifiers(&mut frame)[0].raw += 1;
                technologies(&mut frame)[2].progress_raw += 1;
            }
            // The third country switches technology at its read.
            108 => {
                let row = inputs(&mut frame)
                    .iter_mut()
                    .find(|row| row.country == owner(2))
                    .unwrap();
                row.technology_handle = 2;
                row.database_index = replay_data::rows::Recorded::Value(2);
                row.token = b"synthetic_other".to_vec();
                row.progress_raw = 0;
                let technologies = technologies(&mut frame);
                let at = technologies
                    .partition_point(|row| (row.country, row.database_index) <= (owner(2), 1));
                technologies.insert(
                    at,
                    ResearchTechnologyRow {
                        country: owner(2),
                        database_index: 2,
                        technology_handle: 2,
                        classifier: engine_research::ctechnology::CLASSIFIER_ACTIVE,
                        ..Default::default()
                    },
                );
            }
            // A country field no holder is read from.
            110 => countries(&mut frame)[3].political_power_raw = Some(110),
            // The fourth country gains an empty slot at its read, and loses it.
            112 => {
                let inputs = inputs(&mut frame);
                let at = inputs.partition_point(|row| (row.country, row.index) <= (owner(3), 0));
                inputs.insert(
                    at,
                    ResearchInputRow {
                        country: owner(3),
                        index: 1,
                        ..Default::default()
                    },
                );
            }
            136 => inputs(&mut frame).retain(|row| (row.country, row.index) != (owner(3), 1)),
            // The first country's bonus changes at its read.
            124 => bonuses(&mut frame)[0].speed_bonus_raw += 1,
            // The fifth country's holder is renamed.
            130 => countries(&mut frame)[4].tag = b"VIC".to_vec(),
            // An unobservable hour, holding the fifth country's read.
            140 => {
                stashed = Some(std::mem::take(inputs(&mut frame)));
                *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
                    reason: "synthetic missing research inputs".into(),
                };
            }
            141 => {
                *frame.tables[TableId::ResearchInputs.index()] =
                    TableView::Complete(TableRows::ResearchInputs(stashed.take().unwrap()));
            }
            // After its missed read, the sixth country's technology is done but
            // still in its slot: its progress shows at this read hour alone.
            168 => {
                inputs(&mut frame)
                    .iter_mut()
                    .find(|row| row.country == owner(5))
                    .unwrap()
                    .classifier = engine_research::ctechnology::CLASSIFIER_COMPLETED;
                let technology = technologies(&mut frame)
                    .iter_mut()
                    .find(|row| row.country == owner(5))
                    .unwrap();
                technology.classifier = engine_research::ctechnology::CLASSIFIER_COMPLETED;
                technology.researched = 1;
            }
            // Another read the hour after, so the whole tables observe every
            // country again.
            169 => read_at(&mut frame, 0, 169),
            _ => {}
        }
        let (decoded, wire) = recorded.next(&frame, hour == 126);
        reference.observe_selected(&decoded, None).unwrap();
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
            observer.observe_changed(&decoded, &changes).unwrap();
        }
        if ![113, 135, 160, 172].contains(&hour) {
            continue;
        }
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        let [incremental, whole] = &observers;
        assert_eq!(
            incremental.keys, whole.keys,
            "keys are allocated in one order"
        );
        assert_eq!(
            serde_json::to_value(&incremental.slots).unwrap(),
            serde_json::to_value(&whole.slots).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&incremental.values).unwrap(),
            serde_json::to_value(&whole.values).unwrap(),
            "open values through hour {hour}"
        );
        assert_eq!(
            published(&fixtures[0], incremental),
            published(&fixtures[1], whole),
            "published through hour {hour}"
        );
        for tag in reference.countries.keys() {
            compare(&fixtures[0], &reference, tag);
            compare(&fixtures[1], &reference, tag);
        }
        // Resume both from their checkpoints.
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let (whole_tables, row_hours) = (observer.scan.whole_tables, observer.scan.row_hours);
            *observer = serde_json::from_slice(&serde_json::to_vec(&*observer).unwrap()).unwrap();
            observer.scan.whole_tables = whole_tables;
            observer.scan.row_hours = row_hours;
            tracker.invalidate();
        }
    }
    assert!(
        observers[0].scan.row_hours >= 40,
        "most hours are observed from their changed rows, not {}",
        observers[0].scan.row_hours
    );
    assert_eq!(observers[1].scan.row_hours, 0);
    assert!(
        reference.countries.contains_key("VIC"),
        "the renamed holder is read"
    );
}

#[test]
fn cold_research_keeps_unavailable_slots_instead_of_reporting_no_slots() {
    use crate::test_fixtures::{file_source, source_with_content};
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (_catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/terrain/mod.txt",
        "categories={mod_ground={movement_cost=1}}",
    )]);
    let mut frames = vec![
        sliced_research_frame(100, 100, &[Some(1), None]),
        sliced_research_frame(101, 100, &[Some(1), None]),
    ];
    *frames[1].tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "synthetic missed inputs".into(),
    };
    let (memory, context) = source_with_content(frames.clone(), &folder.0);
    let (source, root) =
        file_source(replay::container::Replay::open(memory.replay_ref().bytes().to_vec()).unwrap());
    let (store, key) = crate::prepared::files(&source, context.content.as_ref()).unwrap();
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({"branchId":"1", "reads":[{"tag":"GER", "hour":101}]});
    let cold = state_query(&source, &context, &params, &store, &key, &flag).unwrap();
    let slots = cold["states"][0]["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|slot| slot["available"] == false));
    assert_eq!(
        cold["states"][0]["data"]["history"]["slots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        cold["states"][0]["data"]["history"]["coverage"][0]["lastHour"],
        100
    );
    assert!(
        cold["states"][0]["data"]["history"]["slots"]
            .as_array()
            .unwrap()
            .iter()
            .all(|slot| slot["states"].as_array().unwrap().is_empty())
    );
    let mut prepared = Prepared::default();
    for frame in &frames {
        prepared.observe(frame).unwrap();
    }
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let exact = values_query("series-state", params, &store, &key, &flag).unwrap();
    let availability = |reply: &serde_json::Value| {
        reply["states"][0]["slots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slot| (slot["index"].clone(), slot["available"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(availability(&cold), availability(&exact));
    store.clear().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn research_equal_projection_carries_numeric_keys_until_publication() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let frame = sliced_research_frame(100, 100, &[Some(1), None]);
    prepared.observe(&frame).unwrap();
    let before = prepared.values.key_visits;
    prepared
        .observe_with(101, |scan| {
            scan.observed = scan.countries.keys().cloned().collect();
            for country in scan.countries.values_mut() {
                country.carry_until = Some(101);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        prepared.values.key_visits, before,
        "equal numeric keys are held"
    );
    prepared.values.seal();
    assert_eq!(
        prepared
            .values
            .state(prepared.keys["GER"])
            .unwrap()
            .last
            .unwrap()
            .last_hour,
        101
    );
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reference = Scan::default();
    reference.observe_selected(&frame, None).unwrap();
    let mut next = frame;
    next.hour = 101;
    reference.observe_selected(&next, None).unwrap();
    reference.seal();
    compare(&fixture, &reference, "GER");
}
