use dashboard_base::test_fixtures::{CAMPAIGN, records, world_frame};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

/// A subject's autonomy: (subject link, overlord link or none, state, score,
/// lower, upper, previous and next exist).
type Status = (u32, Option<u32>, &'static [u8], i64, i64, i64, bool, bool);

#[test]
fn coarse_autonomy_uses_cached_keyframes_with_an_exact_prefix_and_reached_values() {
    use dashboard_base::prepared::{
        coverage::publish_coverage,
        query::{frames, observations, progressive},
    };
    use dashboard_base::test_fixtures::{continuation_records, file_source, replay};
    use dashboard_core::config::LoadContext;
    use serde_json::json;
    let frames = [100, 101, 104, 105].map(|hour| {
        autonomy_frame(
            hour,
            &[(
                31,
                Some(11),
                if hour < 104 { b"low" } else { b"high" },
                i64::from(hour) * 10_000,
                0,
                2_000_000,
                true,
                true,
            )],
        )
    });
    let recording = replay(frames[..2].to_vec());
    let recording = recording
        .append(&[continuation_records(&recording, 1, frames[2..].to_vec())])
        .unwrap();
    let (source, root) = file_source(recording);
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let request = json!({"branchId":"1", "from":100, "to":105, "width":1,
        "within":{"subject":[["D01 mod",100,105,true]]}});
    let ask = |product, value: &serde_json::Value| {
        super::prepared::progressive_query(&source, product, value, &store, &key, &context, &flag)
            .unwrap()
    };
    let cold = ask("series-summary", &request);
    assert_eq!(
        cold["approximate"],
        json!([
            {"firstHour":100,"lastHour":101}, {"firstHour":104,"lastHour":105}
        ])
    );
    assert_eq!(observations::usage(&source, "autonomy:keyframes").2, 2);
    assert_eq!(
        frames::reads(&source),
        (0, 0),
        "a coarse view reads no hourly blocks"
    );
    let (again, refused) = progressive::without_block_decodes(|| ask("series-summary", &request));
    assert!(!refused);
    assert_eq!(again, cold);
    let mut prepared = super::ScannedAutonomy::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for frame in &frames[..2] {
        prepared.prepare(frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    publish_coverage(&mut writer, 1, 100, 101, false).unwrap();
    writer.commit().unwrap();
    let (prefix, refused) = progressive::without_block_decodes(|| ask("series-summary", &request));
    assert!(!refused);
    assert_eq!(prefix["coverage"]["lastHour"], 101);
    assert_eq!(prefix["levels"], cold["levels"]);
    assert_eq!(observations::usage(&source, "autonomy:keyframes").2, 2);
    let reached = ask(
        "series-state",
        &json!({"branchId":"1", "reads":[{"hour":105}]}),
    );
    assert_eq!(reached["exactHours"], json!([105]));
    assert!(
        reached["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row["runs"].as_array().unwrap())
            .any(|run| run["scoreRaw"] == "1050000")
    );
    let mut writer = store.extend(&key).unwrap();
    for frame in &frames[2..] {
        prepared.prepare(frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    publish_coverage(&mut writer, 1, 100, 105, true).unwrap();
    writer.commit().unwrap();
    let exact = ask("series-summary", &request);
    let mut oracle = super::prepared::query(
        &source,
        "series-summary",
        &request,
        &store,
        &key,
        &context,
        &flag,
    )
    .unwrap();
    oracle["coverage"] = exact["coverage"].clone();
    oracle["identities"] = exact["identities"].clone();
    oracle["pending"] = json!([]);
    assert_eq!(exact, oracle);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_subject_between_keyframes_keeps_the_empty_bootstrap_visibly_approximate() {
    use dashboard_base::prepared::{coverage::publish_coverage, query::progressive};
    use dashboard_base::test_fixtures::{continuation_records, file_source, replay};
    use dashboard_core::config::LoadContext;
    use serde_json::json;
    let frames = [100, 101, 102, 104, 105].map(|hour| {
        autonomy_frame(
            hour,
            if hour == 101 {
                &[(31, Some(11), b"between/mod-level", 50, 0, 100, true, true)]
            } else {
                &[]
            },
        )
    });
    let recording = replay(frames[..3].to_vec());
    let recording = recording
        .append(&[continuation_records(&recording, 1, frames[3..].to_vec())])
        .unwrap();
    let (source, root) = file_source(recording);
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let request = json!({"branchId":"1", "from":100, "to":105, "width":1, "within":{}});
    let ask = || {
        super::prepared::progressive_query(
            &source,
            "series-summary",
            &request,
            &store,
            &key,
            &context,
            &flag,
        )
        .unwrap()
    };
    let cold = ask();
    assert_eq!(cold["subjects"], json!([]));
    assert_eq!(cold["bands"], json!([]));
    assert_eq!(cold["summaries"], json!({}));
    assert_eq!(cold["pending"], json!([]));
    assert_eq!(
        cold["approximate"],
        json!([
            {"firstHour":100,"lastHour":102}, {"firstHour":104,"lastHour":105}
        ])
    );
    let (again, refused) = progressive::without_block_decodes(ask);
    assert!(!refused);
    assert_eq!(again, cold);
    let reached = super::prepared::progressive_query(
        &source,
        "series-state",
        &json!({"branchId":"1", "reads":[{"hour":105}]}),
        &store,
        &key,
        &context,
        &flag,
    )
    .unwrap();
    assert_eq!(reached["bands"], json!([]));
    assert_eq!(reached["exactHours"], json!([105]));
    assert_eq!(
        reached["pending"],
        json!([
            {"firstHour":100,"lastHour":102}, {"firstHour":104,"lastHour":105}
        ])
    );
    let mut prepared = super::ScannedAutonomy::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for frame in &frames[..3] {
        prepared.prepare(frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    publish_coverage(&mut writer, 1, 100, 102, false).unwrap();
    writer.commit().unwrap();
    let prefix = ask();
    assert_eq!(prefix["bands"][0]["tag"], "D01 mod");
    assert_eq!(prefix["bands"][0]["runs"][0]["firstHour"], 101);
    assert_eq!(prefix["bands"][0]["runs"][0]["lastHour"], 101);
    assert_eq!(
        prefix["approximate"],
        json!([{"firstHour":104,"lastHour":105}])
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skipped_autonomy_keyframes_leave_every_recorded_unprepared_hour_pending() {
    use dashboard_base::prepared::{coverage::publish_coverage, query::progressive};
    use dashboard_base::test_fixtures::{continuation_records, file_source, replay};
    use dashboard_core::config::LoadContext;
    use serde_json::json;
    let frames = [100, 101, 102, 104, 105].map(|hour| {
        autonomy_frame(
            hour,
            if hour == 101 {
                &[(31, Some(11), b"between/mod-level", 50, 0, 100, true, true)]
            } else {
                &[]
            },
        )
    });
    let recording = replay(frames[..3].to_vec());
    let mut recording = recording
        .append(&[continuation_records(&recording, 1, frames[3..].to_vec())])
        .unwrap();
    for hour in [100, 104] {
        recording.gaps.push(replay_frame::branches::Gap {
            branch: 1,
            first_hour: hour,
            last_hour: hour,
            reason: "synthetic gap".into(),
        });
    }
    let (source, root) = file_source(recording);
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let ask = |product, request: &serde_json::Value| {
        super::prepared::progressive_query(&source, product, request, &store, &key, &context, &flag)
            .unwrap()
    };
    let request = json!({"branchId":"1", "from":100, "to":105, "width":1, "within":{}});
    let cold = ask("series-summary", &request);
    assert_eq!(cold["bands"], json!([]));
    assert!(cold["approximate"].as_array().is_none_or(Vec::is_empty));
    let pending = json!([
        {"firstHour":101,"lastHour":102}, {"firstHour":105,"lastHour":105}
    ]);
    assert_eq!(cold["pending"], pending);
    let reached = ask(
        "series-state",
        &json!({"branchId":"1", "reads":[{"hour":105}]}),
    );
    assert_eq!(reached["bands"], json!([]));
    assert_eq!(reached["exactHours"], json!([105]));
    assert_eq!(reached["pending"], pending);
    let mut bounded_request = request.clone();
    bounded_request["from"] = json!(102);
    bounded_request["to"] = json!(104);
    let (bounded, refused) =
        progressive::without_block_decodes(|| ask("series-summary", &bounded_request));
    assert!(!refused);
    assert_eq!(
        bounded["pending"],
        json!([{"firstHour":102,"lastHour":102}])
    );
    let mut prepared = super::ScannedAutonomy::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for frame in &frames[..3] {
        prepared.prepare(frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    publish_coverage(&mut writer, 1, 100, 102, false).unwrap();
    writer.commit().unwrap();
    let prefix = ask("series-summary", &request);
    assert_eq!(prefix["bands"][0]["runs"][0]["firstHour"], 101);
    assert_eq!(prefix["pending"], json!([{"firstHour":105,"lastHour":105}]));
    let mut writer = store.extend(&key).unwrap();
    for frame in &frames[3..] {
        prepared.prepare(frame).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    publish_coverage(&mut writer, 1, 100, 105, true).unwrap();
    writer.commit().unwrap();
    assert_eq!(ask("series-summary", &request)["pending"], json!([]));
    let _ = std::fs::remove_dir_all(root);
}

/// A world frame (Austria included) with the autonomy and subject tables.
fn autonomy_frame(hour: i32, statuses: &[Status]) -> DecodedFrame {
    use replay_data::{rows::AutonomyRow, rows::SubjectRow};
    let mut frame = world_frame(hour, true);
    let mut statuses = statuses.to_vec();
    // Rows go in key order: Autonomy by subject, Subjects by overlord then subject.
    statuses.sort_by_key(|status| status.0);
    let mut relations = statuses.clone();
    relations.sort_by_key(|status| (status.1, status.0));
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
        statuses
            .iter()
            .map(
                |&(subject, _, state, score, lower_score, upper_score, previous, next)| {
                    AutonomyRow {
                        subject,
                        score,
                        lower_score,
                        upper_score,
                        last_modified_bits: 0,
                        state: state.to_vec(),
                        previous: previous.then(|| b"lower".to_vec()),
                        next: next.then(|| b"higher".to_vec()),
                    }
                },
            )
            .collect(),
    ));
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
        relations
            .iter()
            .filter_map(|&(subject, overlord, state, ..)| {
                overlord.map(|overlord| SubjectRow {
                    overlord,
                    subject,
                    start_hour: 0,
                    end_hour: 0,
                    cancel: 0,
                    autonomy_ratio: 20_000,
                    state: state.to_vec(),
                })
            })
            .collect(),
    ));
    frame
}

#[test]
fn cancelled_and_ended_relations_name_no_overlord() {
    use replay_data::rows::SubjectRow;
    let relation = |overlord: u32, end_hour: i32, cancel: u8| SubjectRow {
        overlord,
        subject: 31,
        start_hour: 0,
        end_hour,
        cancel,
        autonomy_ratio: 20_000,
        state: b"low".to_vec(),
    };
    let frame = |hour: i32| {
        let mut frame = world_frame(hour, true);
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            vec![relation(11, 0, 1), relation(21, 105, 0)],
        ));
        frame
    };
    let live = frame(100);
    let holders = super::Holders::read(&live).unwrap();
    let relations = super::Relations::read(&live, &holders).unwrap();
    assert_eq!(relations.overlords.get(&9), Some(&(8, 20_000)));
    assert!(relations.shared.is_empty());
    assert_eq!(relations.expires, Some(105));
    let relations = super::Relations::read(&frame(105), &holders).unwrap();
    assert!(relations.overlords.is_empty());
    assert_eq!(relations.expires, None);
}

#[test]
fn prepared_autonomy_batches_resume_and_keep_exact_runs_metadata_gaps_and_navigation() {
    use crate::products::autonomy::{AutonomyParams, ScannedAutonomy};
    use dashboard_core::config::LoadContext;
    let frames = [100, 101, 103, 104].map(|hour| {
        autonomy_frame(
            hour,
            &[(
                31,
                Some(11),
                if hour < 104 { b"low" } else { b"high" },
                i64::from(hour - 100) * 100_000,
                0,
                1_000_000,
                true,
                true,
            )],
        )
    });
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap());
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-autonomy".into(),
            revision: "1".into(),
        },
        Default::default(),
        replay.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("autonomy-series-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("source");
    std::fs::write(&path, replay.bytes()).unwrap();
    let key = store::SourceKey::capture(&path, &[]).unwrap();
    let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
    let flag = AtomicBool::new(false);
    let mut scan = ScannedAutonomy::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for frame in &frames[..2] {
        scan.prepare(frame).unwrap();
    }
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let (_, mut scan) = dashboard_base::prepared::series::read_catalogue::<ScannedAutonomy>(
        &store, &key, "autonomy", 1,
    )
    .unwrap()
    .unwrap();
    let mut writer = store.extend(&key).unwrap();
    for frame in &frames[2..] {
        scan.prepare(frame).unwrap();
    }
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        store.range_count(&key, "autonomy-series-1").unwrap(),
        Some(2)
    );
    // A catalogue is its base plus a short chain of patches.
    assert!(
        store
            .range_count(&key, "autonomy-1-catalogue")
            .unwrap()
            .is_some_and(|count| (1..=4).contains(&count))
    );
    for (overlords, subjects) in [
        (vec![], vec![]),
        (vec!["GER".into()], vec![]),
        (vec![], vec!["missing".into()]),
    ] {
        let params = AutonomyParams {
            branch_id: "1".into(),
            overlords,
            subjects,
        };
        let expected = super::scan(&replay, 1, &flag)
            .unwrap()
            .reply(&source, &params, &LoadContext { content: None })
            .unwrap()
            .0;
        let mut request = serde_json::to_value(&params).unwrap();
        request["reads"] = serde_json::json!(
            [100, 101, 102, 103, 104].map(|hour| serde_json::json!({"hour":hour}))
        );
        let actual = super::prepared::query(
            &source,
            "series-state",
            &request,
            &store,
            &key,
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        request["within"] = serde_json::json!({"D01 mod":[["D01 mod",100,104,true]]});
        request["tag"] = serde_json::json!("D01 mod");
        request["from"] = serde_json::json!(100);
        request["firstHour"] = serde_json::json!(100);
        request["lastHour"] = serde_json::json!(104);
        request["key"] = serde_json::json!("ArrowRight");
        let next = super::prepared::query(
            &source,
            "series-next-change",
            &request,
            &store,
            &key,
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        let mut exact = actual.clone();
        for field in ["bands", "summaries", "exactHours", "navigation"] {
            exact.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(exact, serde_json::to_value(expected).unwrap());
        assert_eq!(
            next["hour"],
            if exact["subjects"].as_array().unwrap().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(101)
            }
        );
    }
    store.clear().unwrap();
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn progressive_autonomy_sampler_and_observer_agree_on_every_key() {
    use super::{FIELDS, ScannedAutonomy, sample};
    for count in [3, 12] {
        let mut frame = dashboard_base::test_fixtures::scale_frame(1, 100, count);
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            (1..count)
                .map(|index| replay_data::rows::SubjectRow {
                    subject: 100_000 + index,
                    overlord: 100_000,
                    autonomy_ratio: i64::from(index),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            (1..count)
                .map(|index| replay_data::rows::AutonomyRow {
                    subject: 100_000 + index,
                    state: b"recorded/mod-level".to_vec(),
                    score: -i64::from(index),
                    lower_score: -100_000,
                    upper_score: 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
        let sampled = sample(&frame, None).unwrap();
        let mut joined = ScannedAutonomy::default();
        let before = super::HOLDER_READS.get();
        let actual = joined.observe_sample(&frame, true).unwrap();
        assert_eq!(
            super::HOLDER_READS.get() - before,
            1,
            "numeric and categorical state re-read holders"
        );
        assert_eq!(actual, sampled);
        let mut oracle = ScannedAutonomy::default();
        oracle.observe(&frame).unwrap();
        assert_eq!(
            serde_json::to_vec(&joined).unwrap(),
            serde_json::to_vec(&oracle).unwrap()
        );
        let mut gap = frame.clone();
        gap.hour += 2;
        *gap.tables[TableId::Subjects.index()] = TableView::Unavailable {
            reason: "synthetic refusal".into(),
        };
        assert_eq!(
            joined.observe_sample(&gap, true).unwrap(),
            sample(&gap, None).unwrap()
        );
        oracle.observe(&gap).unwrap();
        let mut resumed = frame.clone();
        resumed.hour += 3;
        assert_eq!(
            joined.observe_sample(&resumed, true).unwrap(),
            sample(&resumed, None).unwrap()
        );
        oracle.observe(&resumed).unwrap();
        assert_eq!(
            serde_json::to_vec(&joined).unwrap(),
            serde_json::to_vec(&oracle).unwrap()
        );

        let mut observer = ScannedAutonomy::default();
        observer.prepare(&frame).unwrap();
        let countries: std::collections::HashMap<_, _> = frame
            .complete::<replay_data::rows::tables::Countries>()
            .unwrap()
            .iter()
            .map(|country| {
                (
                    String::from_utf8(country.tag.clone()).unwrap(),
                    (country.canonical_slot, country.id),
                )
            })
            .collect();
        assert_eq!(sampled.len(), count as usize * FIELDS.len());
        for ((tag, label), &(identity, value)) in &sampled {
            let field = FIELDS.iter().position(|field| *field == label).unwrap() as u64;
            assert_eq!(identity, Some(countries[tag]));
            assert_eq!(
                value,
                observer
                    .by_tag
                    .get(tag)
                    .and_then(|index| observer.batch.state(*index as u64 * 6 + field))
                    .and_then(|state| state.last)
                    .map(|run| run.value)
            );
        }
        let requested = vec![
            (observer.subjects[0].0.clone(), "score".into()),
            ("only in a later tail".into(), "score".into()),
        ];
        let selected = sample(&frame, Some(&requested)).unwrap();
        assert_eq!(selected.len(), requested.len());
        assert_eq!(selected[&requested[1]], (None, None));
        *frame.tables[TableId::Autonomy.index()] = TableView::Unavailable {
            reason: "synthetic failed autonomy read".into(),
        };
        assert!(
            sample(&frame, Some(&requested))
                .unwrap()
                .values()
                .all(|value| value.1.is_none())
        );
    }
}

#[test]
fn autonomy_runs_join_equal_hours_mark_level_changes_and_never_cross_gaps() {
    use crate::products::autonomy::scan;
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let colony: Status = (
        31,
        Some(11),
        b"autonomy_colony",
        300_000_000,
        300_000_000,
        400_000_000,
        true,
        true,
    );
    let risen: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        260_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let released: Status = (
        31,
        None,
        b"autonomy_colony",
        300_000_000,
        300_000_000,
        400_000_000,
        true,
        true,
    );
    // Italy is Germany's subject too, named by Germany's other link, at a modded level.
    let italy: Status = (21, Some(9001), b"modded_level", 10, 0, 100, false, true);
    let mut frames = vec![
        autonomy_frame(100, &[puppet, italy]),
        autonomy_frame(101, &[puppet, italy]),
        autonomy_frame(102, &[risen, italy]),
        autonomy_frame(103, &[colony, italy]),
        // 104 is a gap.
        autonomy_frame(105, &[colony, italy]),
        // Austria's relation ends: no overlord.
        autonomy_frame(106, &[released, italy]),
    ];
    let mut incomplete = autonomy_frame(107, &[released]);
    *incomplete.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    frames.push(incomplete);
    frames.push(autonomy_frame(108, &[released]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.levels,
        ["modded_level", "autonomy_puppet", "autonomy_colony"]
    );
    assert_eq!(scanned.lowest_lower, [0, 200_000_000, 300_000_000]);
    // The gap at 104 and the incomplete hour 107 are not recorded.
    assert_eq!(
        scanned
            .recorded
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103), (105, 106), (108, 108)]
    );
    let (tag, runs) = &scanned.subjects[1];
    assert_eq!(tag, "D01 mod");
    let runs: Vec<_> = runs
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.overlord.as_deref(),
                run.level,
                run.score_raw.as_str(),
                run.ratio_raw.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (100, 101, true, Some("GER"), 1, "250000000", Some("20000")),
            (102, 102, false, Some("GER"), 1, "260000000", Some("20000")),
            (103, 103, false, Some("GER"), 2, "300000000", Some("20000")),
            (105, 105, true, Some("GER"), 2, "300000000", Some("20000")),
            (106, 106, false, None, 2, "300000000", None),
            (108, 108, true, None, 2, "300000000", None),
        ]
    );
    let (tag, runs) = &scanned.subjects[0];
    assert_eq!(tag, "ITA");
    assert_eq!(
        runs.iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.lowest,
                run.highest,
                run.lower_raw.as_str(),
                run.upper_raw.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (100, 103, true, false, "0", "100"),
            (105, 106, true, false, "0", "100")
        ]
    );
}

#[test]
fn autonomy_reply_filters_by_overlord_or_subject_and_orders_levels_by_recorded_bounds_without_content()
 {
    use crate::products::autonomy::{Autonomy, AutonomyParams, AutonomyReply};
    use dashboard_base::products::Product;
    use dashboard_core::config::LoadContext;
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let italy: Status = (21, Some(31), b"modded_level", 10, 0, 100, false, true);
    let frames = vec![
        autonomy_frame(100, &[puppet, italy]),
        autonomy_frame(101, &[puppet, italy]),
    ];
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "au".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = |overlords: &[&str], subjects: &[&str]| {
        Autonomy::load(
            &source,
            &AutonomyParams {
                branch_id: "1".into(),
                overlords: overlords.iter().map(|tag| tag.to_string()).collect(),
                subjects: subjects.iter().map(|tag| tag.to_string()).collect(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let tags = |reply: &AutonomyReply| {
        reply
            .subjects
            .iter()
            .map(|subject| subject.tag.clone())
            .collect::<Vec<_>>()
    };
    let germany = load(&["GER"], &[]);
    assert_eq!(tags(&germany), ["D01 mod"]);
    // Without content the modded level (lowest move-down score 0) comes first.
    assert_eq!(
        germany
            .levels
            .iter()
            .map(|level| (level.id.as_str(), level.name.is_none(), level.defined))
            .collect::<Vec<_>>(),
        [
            ("modded_level", true, false),
            ("autonomy_puppet", true, false)
        ]
    );
    assert_eq!(germany.subjects[0].runs[0].level, 1);
    assert_eq!(germany.score_scale, 100_000);
    assert!(germany.warnings[0].starts_with("Choose the game folders"));
    assert_eq!(tags(&load(&["D01 mod"], &[])), ["ITA"]);
    // In the order they were first recorded as subjects.
    assert_eq!(tags(&load(&[], &["D01 mod", "ITA"])), ["ITA", "D01 mod"]);
    // The branch reply lets the frontend resolve countries across recorded renames.
    assert_eq!(tags(&load(&[], &[])), ["ITA", "D01 mod"]);
    assert!(tags(&load(&["ITA"], &[])).is_empty());
    assert_eq!(germany.recorded.len(), 1);
    assert!(
        Autonomy::validate(&AutonomyParams {
            branch_id: "1".into(),
            overlords: vec![String::new()],
            subjects: vec![]
        })
        .is_err()
    );
}

#[test]
fn autonomy_hours_with_a_subject_or_overlord_the_recording_does_not_name_stay_partly_known() {
    use crate::products::autonomy::{Autonomy, AutonomyParams, scan};
    use dashboard_base::products::Product;
    use dashboard_core::config::LoadContext;
    let puppet: Status = (
        31,
        Some(11),
        b"autonomy_puppet",
        250_000_000,
        200_000_000,
        300_000_000,
        true,
        true,
    );
    let italy: Status = (21, Some(11), b"autonomy_puppet", 10, 0, 100, true, true);
    // Link 77 is in no CountryLinks row: a subject nobody can name.
    let stranger: Status = (77, Some(11), b"autonomy_puppet", 10, 0, 100, true, true);
    // Link 88 is in none either: Italy's overlord cannot be named.
    let orphan: Status = (21, Some(88), b"autonomy_puppet", 10, 0, 100, true, true);
    let frames = vec![
        autonomy_frame(100, &[puppet, stranger]),
        autonomy_frame(101, &[puppet, italy]),
        autonomy_frame(102, &[puppet, orphan]),
    ];
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let spans = |spans: &[dashboard_base::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    // Every hour is recorded, and the two with an unnamed country are marked.
    assert_eq!(spans(&scanned.recorded), [(100, 102)]);
    assert_eq!(spans(&scanned.unidentified), [(100, 100), (102, 102)]);
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "au-unidentified".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = Autonomy::load(
        &source,
        &AutonomyParams {
            branch_id: "1".into(),
            overlords: vec![],
            subjects: vec!["ITA".into()],
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(spans(&reply.unidentified), [(100, 100), (102, 102)]);
    assert!(
        reply.warnings.iter().any(|warning| warning
            .starts_with("Hours with a subject or overlord the recording does not name: 2.")),
        "{:?}",
        reply.warnings
    );
    // Italy is named at 102, without the overlord the recording cannot name.
    let runs: Vec<_> = reply.subjects[0]
        .runs
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.overlord.as_deref()))
        .collect();
    assert_eq!(runs, [(101, 101, Some("GER")), (102, 102, None)]);
}

/// A mod that replaces the autonomy states (its own levels, in another order and
/// number, one of them scripted) orders and names the recorded levels by its own
/// definitions: a level only vanilla defined, or one the mod cannot place, follows
/// the others by its recorded move-down score, and every such level is named.
#[test]
fn autonomy_levels_follow_a_mods_autonomy_states_in_their_order_and_count() {
    use crate::products::autonomy::{ScannedAutonomy, order_levels};
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_mod_autonomy_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, modded) = (root.join("base"), root.join("mod"));
    let states = |levels: &[(&str, &str)]| {
        levels
            .iter()
            .map(|(id, level)| {
                format!("autonomy_state = {{ id = {id} min_freedom_level = {level} }}\n")
            })
            .collect::<String>()
    };
    for (dir, file, text) in [
        (&base, "puppet.txt", states(&[("autonomy_puppet", "0.4")])),
        (
            &base,
            "dominion.txt",
            states(&[("autonomy_dominion", "0.8")]),
        ),
        (
            &modded,
            "serfdom.txt",
            states(&[
                ("autonomy_vassal", "0.5"),
                ("autonomy_serf", "0.1"),
                ("autonomy_scripted", "@LEVEL"),
            ]),
        ),
        (&modded, "allies.txt", states(&[("autonomy_ally", "0.95")])),
    ] {
        let path = dir.join("common/autonomous_states").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let layers = installed_content_core::ResolvedLayers::from_folders(&[
        (&base, &[][..]),
        (&modded, &["common/autonomous_states"][..]),
    ])
    .unwrap();
    let (defined, warnings) = installed_content_core::autonomy::read(&layers).unwrap();
    let mut ids: Vec<_> = defined
        .states
        .iter()
        .map(|state| state.id.as_str())
        .collect();
    ids.sort_unstable();
    // The mod replaced the folder: vanilla's levels are gone, the mod's four remain.
    assert_eq!(
        ids,
        [
            "autonomy_ally",
            "autonomy_scripted",
            "autonomy_serf",
            "autonomy_vassal"
        ]
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].1.contains("autonomy_scripted") && warnings[0].1.contains("not a plain number"),
        "{warnings:?}"
    );
    // Recorded in another order, with a level only vanilla defined.
    let scanned = ScannedAutonomy {
        levels: [
            "autonomy_vassal",
            "autonomy_puppet",
            "autonomy_scripted",
            "autonomy_serf",
            "autonomy_ally",
        ]
        .map(String::from)
        .to_vec(),
        lowest_lower: vec![250, 200, 0, 50, 475],
        ..Default::default()
    };
    let (levels, places) = order_levels(&scanned, Some(&defined), |id| {
        (id == "autonomy_serf").then(|| "Serf".to_owned())
    });
    assert_eq!(
        levels
            .iter()
            .map(|level| (level.id.as_str(), level.name.as_deref(), level.defined))
            .collect::<Vec<_>>(),
        [
            ("autonomy_serf", Some("Serf"), true),
            ("autonomy_vassal", None, true),
            ("autonomy_ally", None, true),
            ("autonomy_scripted", None, false),
            ("autonomy_puppet", None, false),
        ]
    );
    assert_eq!(places, [1, 4, 3, 0, 2]);
    fs::remove_dir_all(&root).unwrap();
}

/// A branch recorded without the Autonomy table is not recorded, which the reply
/// tells apart from a branch where nobody was a subject.
#[test]
fn autonomy_never_recorded_is_not_no_subjects() {
    use crate::products::autonomy::{Autonomy, AutonomyParams};
    use dashboard_base::products::Product;
    use dashboard_core::config::LoadContext;
    let unrecorded = |hour| {
        let mut frame = autonomy_frame(hour, &[]);
        *frame.tables[TableId::Autonomy.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
        frame
    };
    let load = |frames: Vec<DecodedFrame>| {
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
        let source = dashboard_base::source::Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "au".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap();
        Autonomy::load(
            &source,
            &AutonomyParams {
                branch_id: "1".into(),
                overlords: vec!["GER".into()],
                subjects: vec![],
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let missing = load(vec![unrecorded(100), unrecorded(101)]);
    assert!(missing.subjects.is_empty() && missing.recorded.is_empty());
    assert!(
        missing
            .warnings
            .iter()
            .any(|warning| warning.contains("was not recorded in this branch"))
    );
    let none = load(vec![autonomy_frame(100, &[]), autonomy_frame(101, &[])]);
    assert!(none.subjects.is_empty());
    assert_eq!(
        none.recorded
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101)]
    );
}

#[test]
fn autonomy_deferred_runs_match_oracle_after_quiet_spans_two_commits_resume_and_gap() {
    use super::{AutonomyParams, ScannedAutonomy};
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series};
    let flag = AtomicBool::new(false);
    let initial = autonomy_frame(100, &[(31, Some(11), b"level", 40, 0, 100, true, true)]);
    let mut frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let mut changed = initial.clone();
    changed.hour = 113;
    if let TableView::Complete(TableRows::Autonomy(rows)) =
        &mut *changed.tables[TableId::Autonomy.index()]
    {
        rows[0].state = b"next".to_vec();
    }
    frames.push(changed.clone());
    changed.hour = 114;
    frames.push(changed);
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "autonomy-deferred".into(),
            revision: "1".into(),
        },
        Default::default(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap()),
        &flag,
    )
    .unwrap();
    let fixture = series::tests::Fixture::new();
    let mut prepared = ScannedAutonomy::default();
    for batch in [&frames[..6], &frames[6..]] {
        let mut tracker = FrameChangeTracker::new();
        for frame in batch {
            prepared
                .prepare_changed(frame, &tracker.observe(frame, source.replay_ref().format))
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared =
            series::read_catalogue::<ScannedAutonomy>(&fixture.store, &fixture.key, "autonomy", 1)
                .unwrap()
                .unwrap()
                .1;
    }
    let context = dashboard_core::config::LoadContext { content: None };
    let params = AutonomyParams {
        branch_id: "1".into(),
        subjects: vec![],
        overlords: vec![],
    };
    let expected = super::scan(source.replay_ref(), 1, &flag)
        .unwrap()
        .reply(&source, &params, &context)
        .unwrap()
        .0;
    let mut actual = super::prepared::query(&source, "series-state", &serde_json::json!({"branchId":"1","reads":frames.iter().map(|frame| serde_json::json!({"hour":frame.hour})).collect::<Vec<_>>()}), &fixture.store, &fixture.key, &context, &flag).unwrap();
    for field in ["bands", "summaries", "exactHours", "navigation"] {
        actual.as_object_mut().unwrap().remove(field);
    }
    dashboard_base::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
}

// regression: 344eac3d15 fix(dashboard): batch autonomy tail reads and index frame joins
#[test]
fn prepared_autonomy_bulk_tail_reads_reuse_open_bands_after_resume() {
    use crate::products::autonomy::{AutonomyParams, ScannedAutonomy};
    use dashboard_base::{prepared::series, products::LoadContext};
    let flag = AtomicBool::new(false);
    let count = 3;
    let frames = [100, 101].map(|hour| {
        let mut frame = dashboard_base::test_fixtures::scale_frame(1, hour, count);
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            (1..count)
                .map(|index| replay_data::rows::SubjectRow {
                    subject: 100_000 + index,
                    overlord: 100_000,
                    autonomy_ratio: i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            (1..count)
                .map(|index| replay_data::rows::AutonomyRow {
                    subject: 100_000 + index,
                    state: b"mod/state".to_vec(),
                    score: i64::from(hour) * 100_000 + i64::from(index),
                    lower_score: 0,
                    upper_score: 20_000_000,
                    ..Default::default()
                })
                .collect(),
        ));
        frame
    });
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "autonomy-bulk-tail".into(),
            revision: "1".into(),
        },
        Default::default(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()),
        &flag,
    )
    .unwrap();
    let fixture = series::tests::Fixture::new();
    let mut prepared = ScannedAutonomy::default();
    for frame in &frames {
        prepared.prepare(frame).unwrap();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared =
            series::read_catalogue::<ScannedAutonomy>(&fixture.store, &fixture.key, "autonomy", 1)
                .unwrap()
                .unwrap()
                .1;
    }
    let params = AutonomyParams {
        branch_id: "1".into(),
        subjects: Vec::new(),
        overlords: Vec::new(),
    };
    let context = LoadContext { content: None };
    let expected = super::scan(source.replay_ref(), 1, &flag)
        .unwrap()
        .reply(&source, &params, &context)
        .unwrap()
        .0;
    let request =
        serde_json::json!({"branchId":"1", "reads":[{"hour":100},{"hour":101},{"hour":102}]});
    let mut actual = super::prepared::query(
        &source,
        "series-state",
        &request,
        &fixture.store,
        &fixture.key,
        &context,
        &flag,
    )
    .unwrap();
    for field in ["bands", "summaries", "exactHours", "navigation"] {
        actual.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
    assert_eq!(prepared.last.len(), (count - 1) as usize);
    assert!(
        prepared
            .last
            .iter()
            .flatten()
            .all(|last| last.3 == (100, true)),
        "numeric-only changes preserve the open metadata start across resume"
    );
}

#[test]
fn autonomy_numeric_values_resume_after_unknown_ratio_and_zero_width_bounds() {
    use super::ScannedAutonomy;
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        series::{Reader, tests::Fixture},
    };
    let flag = AtomicBool::new(false);
    let first = autonomy_frame(
        100,
        &[(31, Some(11), b"level", 50_000, 0, 100_000, true, true)],
    );
    let mut missing = first.clone();
    missing.hour = 101;
    *missing.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![]));
    if let TableView::Complete(TableRows::Autonomy(rows)) =
        &mut *missing.tables[TableId::Autonomy.index()]
    {
        rows[0].upper_score = 0;
    }
    let mut restored = first.clone();
    restored.hour = 102;
    let fixture = Fixture::new();
    let mut scan = ScannedAutonomy::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in [&first, &missing, &restored] {
        scan.prepare_changed(
            frame,
            &tracker.observe(frame, replay_data::format::FormatVersion::CURRENT),
        )
        .unwrap();
    }
    let index = scan.by_tag["D01 mod"] as u64;
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "autonomy-series-1".into(),
        &flag,
    )
    .unwrap();
    for field in [3, 4] {
        assert!(reader.value_at(index * 6 + field, 100).unwrap().is_some());
        assert!(reader.value_at(index * 6 + field, 101).unwrap().is_none());
        assert!(reader.value_at(index * 6 + field, 102).unwrap().is_some());
    }
}

#[test]
fn autonomy_link_change_reuses_unchanged_country_scores_and_subject_tables() {
    use super::ScannedAutonomy;
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let first = autonomy_frame(
        100,
        &[(31, Some(11), b"level", 50_000, 0, 100_000, true, true)],
    );
    let mut scan = ScannedAutonomy::default();
    let mut tracker = FrameChangeTracker::new();
    let format = replay_data::format::FormatVersion::CURRENT;
    scan.prepare_changed(&first, &tracker.observe(&first, format))
        .unwrap();
    let before = scan.work;
    let mut changed = first.clone();
    changed.hour = 101;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        let at = rows.partition_point(|row| row.id < 99);
        rows.insert(
            at,
            replay_data::rows::CountryLinkRow {
                id: 99,
                canonical_slot: 9,
            },
        );
    }
    scan.prepare_changed(&changed, &tracker.observe(&changed, format))
        .unwrap();
    assert_eq!(
        scan.work.row_visits - before.row_visits,
        changed
            .complete::<replay_data::rows::tables::CountryLinks>()
            .unwrap()
            .len()
    );
    let samples = super::sample(&changed, None).unwrap();
    for (tag, index) in &scan.by_tag {
        for (field, label) in super::FIELDS.into_iter().enumerate() {
            assert_eq!(
                scan.batch
                    .state(*index as u64 * 6 + field as u64)
                    .and_then(|state| state.last)
                    .map(|run| run.value),
                samples[&(tag.clone(), label.to_owned())].1
            );
        }
    }
}

// regression: a756f86eb6 Fix diplomacy identity handling and history limits
#[test]
fn autonomy_accepts_country_selections_at_design_and_stress_scales() {
    use crate::products::autonomy::{Autonomy, AutonomyParams};
    use dashboard_base::products::Product;
    for count in [1_500, 5_000] {
        let tags = (0..count)
            .map(|index| format!("synthetic/{index}"))
            .collect::<Vec<_>>();
        assert!(
            Autonomy::validate(&AutonomyParams {
                branch_id: "1".into(),
                overlords: tags.clone(),
                subjects: tags,
            })
            .is_ok()
        );
    }
}

#[test]
fn autonomy_runs_break_where_another_native_country_holds_the_subjects_slot() {
    use crate::products::autonomy::scan;
    use replay_data::rows::{CountryLinkRow, CountryRow};
    let held = |subject: u32| -> Status {
        (
            subject,
            Some(11),
            b"autonomy_puppet",
            250_000_000,
            200_000_000,
            300_000_000,
            true,
            true,
        )
    };
    let first = autonomy_frame(100, &[held(31)]);
    // At 101 another native country (41) holds `D01 mod`'s slot 9, with equal operands.
    let mut replaced = autonomy_frame(101, &[held(41)]);
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *replaced.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(41, 9, b"D01 mod"),
        country(9001, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *replaced.tables[TableId::CountryLinks.index()] = TableView::Complete(
        TableRows::CountryLinks(vec![link(11, 7), link(21, 8), link(41, 9), link(9001, 7)]),
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, replaced])]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let (tag, runs) = &scanned.subjects[0];
    assert_eq!(tag, "D01 mod");
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect::<Vec<_>>(),
        [(100, 100, true), (101, 101, true)],
        "the replacement starts its own history"
    );
}
