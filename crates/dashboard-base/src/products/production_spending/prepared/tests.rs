use super::super::tests::{applied, day, hour, line};
use super::*;
use crate::prepared::series::tests::Fixture;
use crate::prepared::series::{Accumulator, Run};
use replay::container::Replay;
use replay_data::{
    rows::OutputPurpose, rows::ProductionLineOutputRow, rows::Recorded, rows::TableId,
    rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

/// `row` with its recorded purpose.
pub(crate) fn with(
    mut row: ProductionLineOutputRow,
    purpose: OutputPurpose,
) -> ProductionLineOutputRow {
    row.purpose = Recorded::Value(purpose.raw());
    row
}

/// Prepares `frames` as the production family does: every endpoint and the
/// state flushed in one commit.
pub(crate) fn prepared(frames: &[DecodedFrame], flag: &AtomicBool) -> (Replay, Fixture) {
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames.to_vec())],
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = pages::ProductionAccumulator::new(replay.format.version(), 1, hour(0));
    for frame in frames {
        state.ingest(replay.format, frame, flag).unwrap();
    }
    pages::write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    (replay, fixture)
}

/// The branch's published index, with designs typed through `named`.
pub(crate) fn indexed(fixture: &Fixture, named: Named<'_>) -> (read::Index, read::Classes) {
    let index = read::read_index(&fixture.store, &fixture.key, 1)
        .unwrap()
        .unwrap();
    let classes = read::Classes::new(&index, named);
    (index, classes)
}

fn at(runs: &Value, hour: i32) -> Option<(Value, Value)> {
    runs.as_array()
        .unwrap()
        .iter()
        .find(|run| {
            run["firstHour"].as_i64().unwrap() <= i64::from(hour)
                && i64::from(hour) <= run["lastHour"].as_i64().unwrap()
        })
        .map(|run| (run["totalRaw"].clone(), run["reason"].clone()))
}

#[test]
fn production_spending_reuses_endpoints_after_two_commits_resume_and_append_with_gaps_and_independent_rocket()
 {
    let flag = AtomicBool::new(false);
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    let mut frames = day(0, vec![], Some(vec![]));
    frames.extend(day(
        1,
        vec![line(7, 1, 100_000)],
        Some(vec![applied(7, 1, 1, 800_000)]),
    ));
    let mut final_output = applied(7, 1, 2, 800_003);
    final_output.ended = true;
    frames.extend(day(2, vec![line(7, 1, 100_000)], Some(vec![final_output])));
    frames.extend(day(3, vec![], Some(vec![])));
    let missing = hour(2) + 7;
    for frame in frames
        .iter_mut()
        .filter(|frame| hour(1) <= frame.hour && frame.hour < hour(2))
    {
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            rows[0].display_name = vec![255];
        }
    }
    frames.retain(|frame| frame.hour != missing);
    for frame in frames.iter_mut().filter(|frame| frame.hour == hour(1) + 9) {
        *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable {
            reason: "independent equipment read refused".into(),
        };
    }
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames.clone())],
    )
    .unwrap();
    let oracle = production::scan(&replay, 1, &flag).unwrap();
    let params = ProductionSpendingParams {
        branch_id: "1".into(),
        tag: "opaque|tag".into(),
        refits: false,
    };
    use crate::products::equipment_produced::{
        ProducedCount, ProducedGrouping, ProducedMeasure, ProducedParams,
    };
    let produced_params = ProducedParams {
        branch_id: "1".into(),
        tags: vec![params.tag.clone()],
        top: None,
        measure: ProducedMeasure::Total,
        count: ProducedCount::Ic,
        grouping: ProducedGrouping::Variant,
        included: Vec::new(),
        refits: false,
    };
    let produced_oracle = crate::products::equipment_produced::reply(
        &oracle,
        &produced_params,
        Named::default(),
        Vec::new(),
        &flag,
    )
    .unwrap();
    let expected =
        serde_json::to_value(reply(&oracle, &params, Named::default(), vec![], &flag).unwrap())
            .unwrap();
    let mut fixture = Fixture::new();
    let mut state = pages::ProductionAccumulator::new(replay.format.version(), 1, hour(0));
    let mut writer = fixture.writer();
    let mut endpoint_oracle = Vec::new();
    for frame in &frames {
        state
            .ingest_changed(
                replay.format,
                frame,
                &flag,
                &tracker.observe(frame, replay.format),
            )
            .unwrap();
        if [hour(0) + 23, hour(1) + 23, hour(3) + 23].contains(&frame.hour) {
            pages::write_state(&mut writer, &mut state).unwrap();
            endpoint_oracle.extend(state.completed.drain(..).map(|(page, _)| page));
            writer.commit().unwrap();
            state = pages::latest_state(&fixture.store, &fixture.key, 1)
                .unwrap()
                .unwrap();
            tracker.invalidate();
            if frame.hour == hour(3) + 23 {
                break;
            }
            writer = if frame.hour == hour(1) + 23 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
        }
    }
    let (index, classes) = indexed(&fixture, Named::default());
    assert_eq!(index.endpoints.len(), endpoint_oracle.len());
    for expected_page in &endpoint_oracle {
        crate::products::equipment_produced::tests::assert_published(
            &index,
            &fixture.store,
            &fixture.key,
            expected_page,
        );
        let ordinal = index.endpoint_at(expected_page.first_hour).unwrap();
        for (&variant, kind) in &expected_page.classifications {
            let names = index
                .names_at(&classes, index.variant_id(variant).unwrap(), ordinal)
                .unwrap()
                .expect("a classified variant's names");
            assert_eq!(
                names
                    .iter()
                    .map(|name| (name.first_hour, name.known))
                    .collect::<Vec<_>>(),
                kind.names
                    .iter()
                    .map(|name| (name.first_hour, name.raw.is_some()))
                    .collect::<Vec<_>>(),
                "names of {variant} at {}",
                expected_page.first_hour
            );
        }
    }
    let holder = index.tags[&params.tag];
    let holder_at = |hour: i32| {
        index
            .holder_at(
                &fixture.store,
                &fixture.key,
                holder,
                index.endpoint_at(hour).unwrap(),
            )
            .unwrap()
            .unwrap()
    };
    let empty = holder_at(hour(0) + 23);
    assert_eq!(empty.equipment.as_ref().unwrap().head.present, Some(false));
    assert_eq!(empty.rocket.as_ref().unwrap().head.present, Some(false));
    let rocket_only = holder_at(hour(1) + 9);
    assert!(rocket_only.equipment.is_none());
    assert!(
        rocket_only.rocket.is_some(),
        "equipment refusal cannot discard independent Rocket coverage"
    );
    for date in [
        hour(0),
        hour(1),
        hour(1) + 9,
        hour(2),
        missing,
        hour(3) + 23,
    ] {
        let produced = pages::select::select_with(
            &index,
            &classes,
            &fixture.store,
            &fixture.key,
            date,
            &produced_params,
            0,
            1,
            false,
            &flag,
        )
        .unwrap();
        let expected_produced = produced_oracle.series.first().and_then(|series| {
            let sample = series
                .first_hours
                .iter()
                .zip(&series.last_hours)
                .position(|(&first, &last)| first <= date && date <= last)?;
            crate::products::equipment_produced::tests::produced_decode(
                series.total.as_ref(),
                series.first_hours.len(),
            )[sample]
        });
        let actual_produced = produced
            .and_then(|reply| {
                reply
                    .countries
                    .first()
                    .and_then(|country| country.total.clone())
            })
            .map(|raw| raw.parse::<i64>().unwrap());
        assert_eq!(
            actual_produced, expected_produced,
            "equipment-produced IC at {date}"
        );
        let input = Params {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            refits: false,
            from: None,
            hour: Some(date),
            end: Some(hour(3) + 23),
        };
        let actual = read_pages(
            None,
            &index,
            &classes,
            &fixture.store,
            &fixture.key,
            (hour(0), hour(3) + 23),
            &input,
            date,
            hour(3) + 23,
            Purposes::WithoutRefits,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual["warnings"], expected["warnings"],
            "counted variant and source refusals at {date}"
        );
        for field in ["total", "rocketTotal"] {
            assert_eq!(
                at(&actual[field], date),
                at(&expected[field], date),
                "{field} at {date}"
            );
        }
        for leaf in expected["leaves"].as_array().unwrap() {
            let actual_leaf = actual["leaves"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["variant"] == leaf["variant"] && item["rocket"] == leaf["rocket"])
                .unwrap();
            assert_eq!(at(&actual_leaf["runs"], date), at(&leaf["runs"], date));
        }
    }
}

/// Before production's coverage completes, a reply answers the prepared
/// prefix's endpoints, names the later recorded hours pending and holds them
/// as still being prepared; once complete it is the complete reply.
#[test]
fn production_spending_answers_the_prepared_prefix_with_the_hours_still_pending() {
    let flag = AtomicBool::new(false);
    let mut frames = day(
        0,
        vec![line(7, 1, 100_000)],
        Some(vec![applied(7, 1, 1, 8_000)]),
    );
    frames.extend(day(
        1,
        vec![line(7, 1, 100_000)],
        Some(vec![applied(7, 1, 2, 9_000)]),
    ));
    frames.extend(day(
        2,
        vec![line(7, 1, 100_000)],
        Some(vec![applied(7, 1, 3, 12_000)]),
    ));
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames.clone())],
    )
    .unwrap();
    let format = replay.format;
    let (source, root) = crate::test_fixtures::file_source(replay);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let (prefix_last, last) = (hour(0) + 23, hour(2) + 23);
    let request = |hour: i32| {
        json!({"branchId": "1", "tag": "opaque|tag", "refits": false,
            "from": null, "hour": hour, "end": hour})
    };
    let mut state = pages::ProductionAccumulator::new(format.version(), 1, hour(0));
    // Commits as production prepared on its own commits: through the end of
    // the first day, then through the branch's end.
    let mut commit = |writer: &mut store::Writer, frames: &[DecodedFrame]| {
        for frame in frames {
            state.ingest(format, frame, &flag).unwrap();
        }
        state.flush(writer, &flag).unwrap();
        let through = frames.last().unwrap().hour;
        crate::prepared::coverage::publish_coverage(writer, 1, hour(0), through, through == last)
            .unwrap();
    };
    let mut writer = store.replace(key.clone()).unwrap();
    commit(&mut writer, &frames[..24]);
    writer.commit().unwrap();
    // The old reply waited for complete coverage and answered nothing.
    let cold = query(&source, request(last), None, &flag)
        .unwrap()
        .expect("a prefix reply before completion");
    let mut writer = store.extend(&key).unwrap();
    commit(&mut writer, &frames[24..]);
    writer.commit().unwrap();
    assert_eq!(cold["coverage"]["complete"], false);
    assert_eq!(cold["coverage"]["lastHour"], prefix_last);
    assert_eq!(
        cold["pending"],
        json!([{"firstHour": hour(1), "lastHour": last}])
    );
    assert_eq!(cold["exactHours"], json!([prefix_last]));
    assert_eq!(
        cold["approximate"],
        json!([{"firstHour": hour(0), "lastHour": prefix_last}])
    );
    // The prefix's amounts are those the complete reply answers at that hour.
    let complete = query(&source, request(prefix_last), None, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(complete["coverage"]["complete"], true);
    assert_eq!(complete["pending"], json!([]));
    assert_eq!(complete["approximate"], json!([]));
    assert!(at(&cold["total"], prefix_last).unwrap().0.is_string());
    for field in ["total", "rocketTotal"] {
        assert_eq!(
            at(&cold[field], prefix_last),
            at(&complete[field], prefix_last),
            "{field}"
        );
        // Every later hour is still being prepared, never an unread hour.
        assert_eq!(
            at(&cold[field], last),
            Some((Value::Null, json!("preparing"))),
            "{field}"
        );
    }
    for leaf in complete["leaves"].as_array().unwrap() {
        let cold_leaf = cold["leaves"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["variant"] == leaf["variant"] && item["rocket"] == leaf["rocket"])
            .unwrap();
        assert_eq!(
            at(&cold_leaf["runs"], prefix_last),
            at(&leaf["runs"], prefix_last)
        );
        assert_eq!(
            at(&cold_leaf["runs"], last),
            Some((Value::Null, json!("preparing")))
        );
    }
    // Once complete, the last hour is answered as the branch scan answers it.
    let oracle = serde_json::to_value(
        reply(
            &production::scan(source.replay_ref(), 1, &flag).unwrap(),
            &ProductionSpendingParams {
                branch_id: "1".into(),
                tag: "opaque|tag".into(),
                refits: false,
            },
            Named::default(),
            Vec::new(),
            &flag,
        )
        .unwrap(),
    )
    .unwrap();
    let whole = query(&source, request(last), None, &flag).unwrap().unwrap();
    assert_eq!(whole["pending"], json!([]));
    assert_eq!(at(&whole["total"], last), at(&oracle["total"], last));
    drop(source);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn production_spending_extrema_clip_partial_ranges_and_resolve_wide_integer_ties() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut values = Accumulator::default();
    let wide = 1i128 << 80;
    for (hour, value) in [
        (0, wide + 100),
        (1, wide),
        (2, wide + 1),
        (3, wide - 1),
        (4, wide + 200),
    ] {
        values.push(0, Run::new(hour, hour, value, false));
    }
    values
        .flush(&mut writer, "production-spent-1", 0, &flag)
        .unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "production-spent-1".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(maximum(&mut reader, 0, 1, 3).unwrap(), Some(wide + 1));
    assert_eq!(maximum(&mut reader, 0, 0, 4).unwrap(), Some(wide + 200));
}

#[test]
fn production_spending_shared_runs_keep_new_refit_and_rocket_source_extrema_distinct() {
    let flag = AtomicBool::new(false);
    let refit = |line, units, spent| with(applied(line, 1, units, spent), OutputPurpose::Refit);
    let lines = vec![line(7, 1, 100_000), line(8, 1, 100_000)];
    let mut frames = day(
        0,
        lines.clone(),
        Some(vec![applied(7, 1, 1, 8_000), refit(8, 1, -3_000)]),
    );
    frames.extend(day(
        1,
        lines,
        Some(vec![applied(7, 1, 2, 9_000), refit(8, 2, -5_000)]),
    ));
    for frame in &mut frames {
        let later = frame.hour >= hour(1);
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            rows.push(replay_data::rows::RocketProductionOutputRow {
                country: 1,
                variant: 1,
                since_hour: hour(0),
                ticks: if later { 2 } else { 1 },
                deliveries: if later { 2 } else { 1 },
                spent_raw: if later { 2_500 } else { 2_000 },
                units: if later { 2 } else { 1 },
                exact: true,
                units_exact: true,
                ..Default::default()
            });
        }
    }
    let (replay, fixture) = prepared(&frames, &flag);
    let (index, classes) = indexed(&fixture, Named::default());
    let oracle = production::scan(&replay, 1, &flag).unwrap();
    for refits in [false, true] {
        let params = ProductionSpendingParams {
            branch_id: "1".into(),
            tag: "opaque|tag".into(),
            refits,
        };
        let expected = serde_json::to_value(
            reply(&oracle, &params, Named::default(), Vec::new(), &flag).unwrap(),
        )
        .unwrap();
        let input = Params {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            refits,
            from: None,
            hour: Some(hour(1) + 23),
            end: Some(hour(1) + 23),
        };
        let actual = read_pages(
            None,
            &index,
            &classes,
            &fixture.store,
            &fixture.key,
            (hour(0), hour(1) + 23),
            &input,
            hour(1) + 23,
            hour(1) + 23,
            Purposes::counting(refits),
            &flag,
        )
        .unwrap();
        assert_eq!(
            at(&actual["total"], hour(1) + 23),
            at(&expected["total"], hour(1) + 23)
        );
        assert_eq!(
            actual["peaks"][serde_json::to_string(&("1", false)).unwrap()],
            if refits { "5000" } else { "9000" }
        );
        assert_eq!(
            actual["peaks"][serde_json::to_string(&("1", true)).unwrap()],
            "2500"
        );
    }
}

#[test]
fn production_spending_split_restores_converted_endpoints_and_leaves_their_peak_out() {
    let flag = AtomicBool::new(false);
    // Line 7 builds design 1 new, then converts to it; line 9 builds design 2.
    let converted = |units, spent| {
        let mut row = with(applied(7, 1, units, spent), OutputPurpose::Converted);
        row.ordinal = 1;
        row
    };
    let lines = vec![line(7, 1, 100_000), line(9, 2, 100_000)];
    let mut frames = day(
        0,
        lines.clone(),
        Some(vec![applied(7, 1, 1, 8_000), applied(9, 2, 1, 4_000)]),
    );
    frames.extend(day(
        1,
        lines,
        Some(vec![
            applied(7, 1, 1, 8_000),
            converted(2, 6_000),
            applied(9, 2, 2, 7_000),
        ]),
    ));
    let (replay, fixture) = prepared(&frames, &flag);
    let (index, classes) = indexed(&fixture, Named::default());
    let oracle = production::scan(&replay, 1, &flag).unwrap();
    let params = ProductionSpendingParams {
        branch_id: "1".into(),
        tag: "opaque|tag".into(),
        refits: true,
    };
    let expected = serde_json::to_value(
        reply_inner(
            &oracle,
            &params,
            Named::default(),
            Vec::new(),
            &flag,
            None,
            Purposes::Split,
        )
        .unwrap(),
    )
    .unwrap();
    let last = hour(1) + 23;
    let input = Params {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        refits: true,
        from: None,
        hour: Some(last),
        end: Some(last),
    };
    let actual = read_pages(
        None,
        &index,
        &classes,
        &fixture.store,
        &fixture.key,
        (hour(0), last),
        &input,
        last,
        last,
        Purposes::Split,
        &flag,
    )
    .unwrap();
    let leaves = |reply: &Value| {
        reply["leaves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|leaf| {
                let value = at(&leaf["runs"], last).map(|(total, _)| total.to_string());
                (leaf["variant"].to_string(), leaf["purpose"].to_string(), value)
            })
            .collect::<BTreeSet<_>>()
    };
    let restored = leaves(&actual);
    assert_eq!(restored, leaves(&expected));
    let leaf = |variant: &str, purpose: &str, total: &str| {
        (
            json!(variant).to_string(),
            json!(purpose).to_string(),
            Some(json!(total).to_string()),
        )
    };
    assert_eq!(
        restored,
        BTreeSet::from([
            leaf("1", "new", "8000"),
            leaf("1", "converted", "6000"),
            leaf("2", "new", "7000"),
        ])
    );
    // The series without refits of design 1 also holds its conversions, so
    // its new leaf has no peak; design 2's has its own.
    let peak = |variant: &str| {
        actual["peaks"][serde_json::to_string(&(variant, false)).unwrap()].clone()
    };
    assert_eq!(peak("1"), Value::Null);
    assert_eq!(peak("2"), json!("7000"));
}

