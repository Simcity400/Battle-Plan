use super::super::tests::{applied, day, hour, line};
use super::*;
use crate::prepared::series::tests::Fixture;
use crate::prepared::series::{Accumulator, Run};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{TableId, TableRows},
};

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
        if let Some(page) = state
            .ingest_changed(
                replay.format,
                frame,
                Named::default(),
                &flag,
                &tracker.observe(frame, replay.format),
            )
            .unwrap()
        {
            pages::publish_endpoint(&mut writer, &mut state, &page).unwrap();
            endpoint_oracle.push(page);
        }
        if [hour(0) + 23, hour(1) + 23, hour(3) + 23].contains(&frame.hour) {
            if let Some(page) = state.flush() {
                pages::publish_endpoint(&mut writer, &mut state, &page).unwrap();
                endpoint_oracle.push(page);
            }
            pages::write_state(&mut writer, &mut state).unwrap();
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
    let count = pages::endpoint_count(&fixture.store, &fixture.key, 1).unwrap();
    for expected_page in endpoint_oracle {
        for date in [expected_page.first_hour, expected_page.last_hour] {
            let restored = pages::read_at(&fixture.store, &fixture.key, 1, count, date, &flag)
                .unwrap()
                .unwrap();
            assert_eq!(restored.classifications, expected_page.classifications);
            // Later coverage repairs can invalidate an earlier equipment/Rocket
            // join; the independent native source endpoints stay identical.
            for (actual, expected) in restored.countries.iter().zip(&expected_page.countries) {
                assert_eq!(actual.tag, expected.tag);
                assert_eq!(
                    actual.equipment, expected.equipment,
                    "equipment operands at {date}"
                );
                assert_eq!(actual.rocket, expected.rocket, "Rocket operands at {date}");
            }
            assert_eq!(restored.countries.len(), expected_page.countries.len());
        }
    }
    let empty = pages::read_at(&fixture.store, &fixture.key, 1, count, hour(0) + 23, &flag)
        .unwrap()
        .unwrap();
    let row = country(&empty, &params.tag).unwrap();
    assert_eq!(row.equipment.as_ref().unwrap().present, Some(false));
    assert_eq!(row.rocket.as_ref().unwrap().present, Some(false));
    let rocket_only = pages::read_at(&fixture.store, &fixture.key, 1, count, hour(1) + 9, &flag)
        .unwrap()
        .unwrap();
    let row = country(&rocket_only, &params.tag).unwrap();
    assert!(row.equipment.is_none());
    assert!(
        row.rocket.is_some(),
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
        let produced = pages::select_at(
            &fixture.store,
            &fixture.key,
            date,
            &produced_params,
            0,
            1,
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
            &fixture.store,
            &fixture.key,
            1,
            (hour(0), hour(3) + 23),
            &input,
            date,
            hour(3) + 23,
            &flag,
        )
        .unwrap()
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
    let mut refit = line(8, 1, 100_000);
    refit.kind = engine::cequipment_production_line::KIND_SHIP_REFIT;
    let mut frames = day(
        0,
        vec![line(7, 1, 100_000), refit.clone()],
        Some(vec![applied(7, 1, 1, 8_000), applied(8, 1, 1, -3_000)]),
    );
    frames.extend(day(
        1,
        vec![line(7, 1, 100_000), refit],
        Some(vec![applied(7, 1, 2, 9_000), applied(8, 1, 2, -5_000)]),
    ));
    for frame in &mut frames {
        let later = frame.hour >= hour(1);
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            rows.push(replay::rows::RocketProductionOutputRow {
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
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames.clone())],
    )
    .unwrap();
    let oracle = production::scan(&replay, 1, &flag).unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = pages::ProductionAccumulator::new(replay.format.version(), 1, hour(0));
    for frame in &frames {
        if let Some(page) = state
            .ingest(replay.format, frame, Named::default(), &flag)
            .unwrap()
        {
            pages::publish_endpoint(&mut writer, &mut state, &page).unwrap();
        }
    }
    let page = state.flush().unwrap();
    pages::publish_endpoint(&mut writer, &mut state, &page).unwrap();
    pages::write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
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
            &fixture.store,
            &fixture.key,
            1,
            (hour(0), hour(1) + 23),
            &input,
            hour(1) + 23,
            hour(1) + 23,
            &flag,
        )
        .unwrap()
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

#[cfg(feature = "slow-tests")]
#[test]
fn production_spending_preserves_all_5000_country_endpoints_and_sparse_leaf_keys() {
    use replay::rows::{CountryLinkRow, CountryRow, ProductionLineOutputRow, ProductionLineRow};
    let flag = AtomicBool::new(false);
    let mut frame = day(0, vec![], Some(vec![])).remove(0);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id:04}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5000)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            (1..=5000)
                .map(|id| ProductionLineRow {
                    handle: u64::from(id),
                    country: id,
                    variant: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(
            (1..=5000)
                .map(|id| ProductionLineOutputRow {
                    country: id,
                    ..applied(u64::from(id), 1, 1, 700_000)
                })
                .collect(),
        ));
    let mut state = pages::ProductionAccumulator::new(replay::FORMAT_VERSION, 1, frame.hour);
    state
        .ingest(
            replay::format::FormatVersion::read(replay::FORMAT_VERSION).unwrap(),
            &frame,
            Named::default(),
            &flag,
        )
        .unwrap();
    let page = state.flush().unwrap();
    assert_eq!(page.countries.len(), 5000);
    assert!(
        page.countries
            .iter()
            .all(|row| row.equipment.as_ref().unwrap().present == Some(true))
    );
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    pages::publish_endpoint(&mut writer, &mut state, &page).unwrap();
    pages::write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    let restored = pages::read_at(&fixture.store, &fixture.key, 1, 1, frame.hour, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(restored, page);
    let catalogue = pages::read_catalogue(&fixture.store, &fixture.key, 1)
        .unwrap()
        .unwrap();
    assert_eq!(catalogue.spent_keys.len(), 10_000);
    eprintln!(
        "production scale: 5000 countries, {} leaf keys; {} continuation bytes",
        catalogue.spent_keys.len(),
        serde_json::to_vec(&state).unwrap().len()
    );
}
