//! Stability, war support and maximum command power answer beyond prepared
//! coverage through the shared progressive reader, as preparation answers.
use super::*;
use crate::{
    prepared_stability::StabilityAccumulator,
    test_fixtures::{stability_frame, stability_inputs},
};
use replay_data::rows::CountryCommandPowerRow;
use serde_json::Value;

const MEASURES: [&str; 3] = ["stability", "warSupport", "maxCommandPower"];

/// GER's inputs with a carried read, an uncomputable stability, a dropped
/// inputs table, a missing command power row, two missing hours, a replaced
/// native holder and its rename, then a long renamed stretch whose reads
/// alternate between fresh and carried.
fn frames() -> Vec<DecodedFrame> {
    let command = |frame: &mut DecodedFrame, present: bool| {
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(
                present
                    .then(|| CountryCommandPowerRow {
                        canonical_slot: 7,
                        allocated_command_power_raw: 100_000,
                        max_command_power: Some(500_000),
                        max_command_power_factor: Some(10_000),
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
            ));
    };
    let inputs = stability_inputs();
    let mut unknown = inputs.clone();
    unknown.stability_factor = None;
    unknown.keys[0] = u32::MAX;
    let mut changed = inputs.clone();
    changed.stability_raw = 60_000;
    let mut dropped = stability_frame(3, inputs.clone(), 2, vec![]);
    *dropped.tables[TableId::CountryStabilityInputs.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut frames = vec![
        stability_frame(0, inputs.clone(), 0, vec![]),
        stability_frame(1, inputs.clone(), 0, vec![]),
        stability_frame(2, unknown, 2, vec![]),
        dropped,
        stability_frame(4, inputs.clone(), 4, vec![]),
        stability_frame(5, changed.clone(), 4, vec![]),
        stability_frame(7, changed, 7, vec![]),
    ];
    for (index, frame) in frames.iter_mut().enumerate() {
        command(frame, index != 4);
    }
    let replaced = |hour: i32, tag: &[u8], stability: i64, read: i32| {
        let mut row = inputs.clone();
        row.country = 9100;
        row.stability_raw = stability;
        let mut frame = stability_frame(hour, row, read, vec![]);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            let holder = rows.iter_mut().find(|row| row.tag == b"GER").unwrap();
            holder.id = 9100;
            holder.tag = tag.to_vec();
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.iter_mut().find(|row| row.id == 9001).unwrap().id = 9100;
        }
        if let TableView::Complete(TableRows::SliceReads(rows)) =
            &mut *frame.tables[TableId::SliceReads.index()]
        {
            rows.iter_mut().for_each(|row| row.owner = 9100);
        }
        command(&mut frame, true);
        frame
    };
    frames.push(replaced(9, &b"GER"[..], 60_000, 9));
    for hour in 10..42 {
        frames.push(replaced(
            hour,
            &b"renamed"[..],
            50_000 + i64::from(hour / 3) * 100,
            hour - hour % 2,
        ));
    }
    frames
}

fn summary(measure: &str, tags: &[&str], from: i32, to: i32) -> Value {
    serde_json::json!({"branchId":"1","family":{"kind":"country-stability","measure":measure},
        "tags":tags,"from":from,"to":to,"width":100})
}

/// A summary's lines with run extents clipped to the view: a transient read
/// knows a run only as far as the frames it reconstructed, while every
/// value, bucket and break inside the view must be identical.
fn in_view(mut series: Value, from: i64, to: i64) -> Value {
    for line in series.as_array_mut().unwrap() {
        for run in line["summary"]["runs"].as_array_mut().unwrap() {
            run["firstHour"] = run["firstHour"].as_i64().unwrap().max(from).into();
            run["lastHour"] = run["lastHour"].as_i64().unwrap().min(to).into();
        }
    }
    series
}

/// Every value a state reply dates: the run's bounds are the reached hour's
/// own evidence on a transient read, so they are not part of the comparison.
fn state_values(state: &Value) -> Value {
    serde_json::json!({
        "tag": state["tag"], "hour": state["hour"], "approximate": state["approximate"],
        "readHour": state["readHour"], "valueRaw": state["run"]["valueRaw"],
        "slot": state["run"]["slot"], "countryId": state["run"]["countryId"],
        "runReadHour": state["run"]["readHour"],
    })
}

#[test]
fn stability_measures_answer_unprepared_spans_from_the_shared_sampler_as_preparation_does() {
    independent::with_test_hourly(stability_measures_answer_unprepared_spans_from_the_shared_sampler_as_preparation_does_hourly);
}

/// The stability pages are written as the hourly walk publishes them.
fn stability_measures_answer_unprepared_spans_from_the_shared_sampler_as_preparation_does_hourly() {
    let frames = frames();
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap();
    let mut blocks = vec![&frames[2..4], &frames[4..8]];
    blocks.extend(frames[8..].chunks(2));
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, block.to_vec())])
            .unwrap();
    }
    assert_eq!(
        replay
            .index
            .iter()
            .filter(|block| block.branch == 1)
            .count(),
        19
    );
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive stability");
    let flag = AtomicBool::new(false);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let query = |product: &str, params: Value| -> Value {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: Value = serde_json::from_str(
            &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let reads = |measure: &str| {
        let reads: Vec<_> = ["GER", "renamed"]
            .into_iter()
            .flat_map(|tag| (0..=13).map(move |hour| serde_json::json!({"tag":tag,"hour":hour})))
            .collect();
        serde_json::json!({"branchId":"1","family":{"kind":"country-stability","measure":measure},
            "reads":reads})
    };
    let navigations = [
        ("ArrowRight", 0),
        ("ArrowRight", 4),
        ("ArrowRight", 9),
        ("ArrowLeft", 13),
        ("ArrowLeft", 7),
    ];
    // What the stability and command power widgets ask at open, plus a zoom
    // onto the renamed holder's stretch and keyboard steps over the window.
    let observe = || -> Vec<Value> {
        let mut replies = Vec::new();
        for measure in MEASURES {
            for (tag, from) in [("GER", 0), ("renamed", 10)] {
                replies.push(in_view(
                    query("series-summary", summary(measure, &[tag], from, 13))["series"].clone(),
                    i64::from(from),
                    13,
                ));
            }
            replies.push(Value::Array(
                query("series-state", reads(measure))["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(state_values)
                    .collect(),
            ));
        }
        for (key, from) in navigations {
            let reply = query(
                "series-next-change",
                serde_json::json!({"branchId":"1","family":{"kind":"country-stability","measure":"stability"},
                    "tags":["GER","renamed"],"measures":MEASURES,"key":key,"from":from,
                    "firstHour":0,"lastHour":13}),
            );
            replies.push(serde_json::json!([reply["hour"], reply["approximate"]]));
        }
        replies
    };

    // Nothing prepared: the gated family used to answer pending (null).
    let cold = observe();
    let first = query("series-summary", summary("stability", &["GER"], 0, 13));
    assert!(
        first["coverage"]["firstHour"]
            .as_i64()
            .is_some_and(|hour| first["coverage"]["lastHour"].as_i64() == hour.checked_sub(1))
            && first["coverage"]["complete"] == false
    );
    let line = &first["series"][0]["summary"];
    assert_eq!(line["approximate"], serde_json::json!([]));
    assert_eq!(
        line["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| (
                run["firstHour"].as_i64().unwrap(),
                run["lastHour"].as_i64().unwrap()
            ))
            .collect::<Vec<_>>(),
        [(0, 1), (4, 4), (5, 5), (7, 7), (9, 9)],
        "unknown, dropped, missing and replaced hours end the exact runs"
    );
    let state = query("series-state", reads("stability"));
    let read = state["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["tag"] == "GER" && state["hour"] == 5)
        .unwrap();
    assert_eq!(
        read["readHour"], 4,
        "a carried read keeps its supplying hour"
    );
    assert_eq!(read["run"]["readHour"], 4);
    assert_eq!(read["recorded"], true);
    // A wide view samples one keyframe per block and says so.
    let rough = query(
        "series-summary",
        summary("stability", &["GER", "renamed"], 0, 41),
    );
    assert!(
        !rough["series"][1]["summary"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !rough["series"][1]["summary"]["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "direct reads never create prepared files"
    );

    // A published prefix joins the exact tail at its coverage edge.
    let mut accumulator = StabilityAccumulator::default();
    for frame in &frames[..4] {
        accumulator.observe(frame).unwrap();
    }
    let mut writer = store.replace(key.clone()).unwrap();
    accumulator.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 3,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "stability-1").unwrap();
    let prefixed = observe();
    assert_eq!(
        query("series-summary", summary("stability", &["GER"], 0, 13))["coverage"]["lastHour"],
        3
    );
    assert_eq!(
        store.range_count(&key, "stability-1").unwrap(),
        ranges,
        "tail reads write nothing"
    );

    // Complete preparation through the production observer.
    accumulator = serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
    for frame in &frames[4..] {
        accumulator.observe(frame).unwrap();
    }
    let mut writer = store.extend(&key).unwrap();
    accumulator.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 41,
            complete: true,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    assert_eq!(
        query("series-summary", summary("stability", &["GER"], 0, 13))["coverage"]["complete"],
        true
    );
    let prepared = observe();
    assert_eq!(cold.len(), prepared.len());
    for (index, (direct, expected)) in cold.iter().zip(&prepared).enumerate() {
        assert_eq!(direct, expected, "direct reply {index}");
    }
    for (index, (joined, expected)) in prefixed.iter().zip(&prepared).enumerate() {
        assert_eq!(joined, expected, "prefix and tail reply {index}");
    }
}

#[test]
fn stability_sampler_matches_the_branch_scan_for_every_measure_at_5000_countries() {
    use crate::{
        prepared_stability::sample, products::country_stability, test_fixtures::scale_tag,
    };
    const COUNT: u32 = 5_000;
    let frame = |hour: i32| {
        let mut frame = scale_frame(1, hour, COUNT);
        *frame.tables[TableId::CountryStabilityInputs.index()] =
            TableView::Complete(TableRows::CountryStabilityInputs(
                (0..COUNT)
                    .map(|index| {
                        let mut row = stability_inputs();
                        row.country = 100_000 + index;
                        row.stability_raw = i64::from(index) * 7 + i64::from(hour);
                        row
                    })
                    .collect(),
            ));
        *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
            (0..COUNT)
                .map(|index| replay_data::rows::SliceReadRow {
                    owner: 100_000 + index,
                    table: TableId::CountryStabilityInputs as u32,
                    sample_hour: hour - i32::from(index % 2 == 1),
                })
                .collect(),
        ));
        // Every third country has no command power row: its maximum is unknown.
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(
                (0..COUNT)
                    .filter(|index| index % 3 != 0)
                    .map(|index| CountryCommandPowerRow {
                        canonical_slot: index,
                        allocated_command_power_raw: i64::from(index),
                        max_command_power: Some(500_000),
                        max_command_power_factor: Some(10_000),
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame
    };
    let frames = vec![frame(100), frame(101)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let tags: Vec<String> = (0..COUNT)
        .map(|index| String::from_utf8(scale_tag(index)).unwrap())
        .collect();
    let (oracle, _) = country_stability::scan(
        &replay,
        1,
        &tags.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    for frame in &frames {
        let sampled = sample(frame, None).unwrap();
        assert_eq!(sampled.len(), COUNT as usize * 4);
        for (index, tag) in tags.iter().enumerate() {
            let series = &oracle[tag];
            for (measure, runs) in [
                ("stability", &series.stability),
                ("warSupport", &series.war_support),
                ("maxCommandPower", &series.max_command_power),
            ] {
                let expected = runs
                    .iter()
                    .find(|run| run.first_hour <= frame.hour && frame.hour <= run.last_hour)
                    .map(|run| run.value_raw.parse::<i128>().unwrap());
                let (identity, value) = sampled[&(tag.clone(), measure.to_owned())];
                assert_eq!(value, expected, "{tag} {measure} {}", frame.hour);
                assert_eq!(
                    identity,
                    expected.map(|_| (index as u32, 100_000 + index as u32))
                );
            }
            let read = series
                .carried
                .iter()
                .find(|read| read.first_hour <= frame.hour && frame.hour <= read.last_hour)
                .map_or(frame.hour, |read| read.read_hour);
            assert_eq!(
                sampled[&(tag.clone(), "readHour".to_owned())].1,
                Some(i128::from(read))
            );
        }
    }
    // A requested country samples only its own keys and its supplying read.
    let one = sample(&frames[0], Some(&[(tags[1].clone(), "warSupport".into())])).unwrap();
    assert_eq!(
        one.keys().cloned().collect::<Vec<_>>(),
        [
            (tags[1].clone(), "readHour".to_owned()),
            (tags[1].clone(), "warSupport".to_owned())
        ]
    );
    assert!(sample(&frames[0], Some(&[(tags[0].clone(), "army-banked".into())])).is_err());
}
