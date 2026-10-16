use super::*;

fn observer_frame(countries: u32, invalid_tag: bool) -> DecodedFrame {
    use replay::rows::{
        CountryLinkRow, CountryRow, ProductionLineOutputRow, ProductionLineRow,
        RocketProductionOutputRow, TableRows,
    };
    let mut frame = crate::test_fixtures::source(1, 8, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: if invalid_tag && id == countries {
                    vec![255]
                } else {
                    format!("mod country {id}").into_bytes()
                },
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
            handle: 1,
            country: 1,
            variant: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(vec![
            ProductionLineOutputRow {
                line: 1,
                country: 1,
                variant: 1,
                units: 1,
                since_hour: 8,
                exact: true,
                spent_raw: Recorded::Value(Some(7)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::RocketProductionOutput.index()] =
        TableView::Complete(TableRows::RocketProductionOutput(vec![
            RocketProductionOutputRow {
                since_hour: 8,
                exact: true,
                units_exact: true,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(Vec::new()));
    *frame.tables[TableId::VariantModules.index()] =
        TableView::Complete(TableRows::VariantModules(Vec::new()));
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(Vec::new()));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(Vec::new()));
    frame
}

fn observer_format(frame: &DecodedFrame) -> FormatVersion {
    replay::container::Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, vec![frame.clone()])],
    )
    .unwrap()
    .format
}

#[test]
fn production_observer_work_is_flat_over_carried_hours_and_retired_holders() {
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let flag = AtomicBool::new(false);
    let first = observer_frame(2, true);
    let format = observer_format(&first);
    observer_tests::assert_flat(
        observer_tests::scaled_work(1, |hours| {
            let mut state = ProductionAccumulator::new(format.version(), 1, first.hour);
            let mut tracker = FrameChangeTracker::new();
            state.ingest_changed(
                format,
                &first,
                Named::default(),
                &flag,
                &tracker.observe(&first, format),
            )?;
            observer_tests::unchanged_frames(
                state.work(),
                &first,
                hours as i32,
                format,
                |frame, changes| {
                    state.ingest_changed(format, frame, Named::default(), &flag, changes)?;
                    Ok(state.work())
                },
            )
        })
        .unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |history| {
            let mut state = ProductionAccumulator::new(format.version(), 1, first.hour);
            let mut tracker = FrameChangeTracker::new();
            let mut frame = first.clone();
            frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
            for alias in 0..history {
                frame.hour = first.hour + alias as i32;
                if let TableView::Complete(replay::rows::TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = format!("retired production holder {alias}").into_bytes();
                }
                state.ingest_changed(
                    format,
                    &frame,
                    Named::default(),
                    &flag,
                    &tracker.observe(&frame, format),
                )?;
            }
            let before = state.work();
            frame.hour += 1;
            if let TableView::Complete(replay::rows::TableRows::ProductionLineOutput(rows)) =
                &mut *frame.tables[TableId::ProductionLineOutput.index()]
            {
                rows[0].units = 2;
                rows[0].spent_raw = Recorded::Value(Some(9));
            }
            state.ingest_changed(
                format,
                &frame,
                Named::default(),
                &flag,
                &tracker.observe(&frame, format),
            )?;
            let after = state.work();
            Ok(Work {
                row_visits: after.row_visits - before.row_visits,
                history_visits: after.history_visits - before.history_visits,
                emitted: after.emitted - before.emitted,
                ..Default::default()
            })
        })
        .unwrap(),
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn production_endpoint_warning_clock_matches_every_oracle_hour_across_two_commits_resume_append_and_gaps()
 {
    use crate::prepared::series::tests::Fixture;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::rows::TableRows;
    let flag = AtomicBool::new(false);
    let initial = observer_frame(2, true);
    let format = observer_format(&initial);
    let mut frames = Vec::new();
    for offset in 0..56 {
        if offset == 17 {
            continue;
        }
        let mut frame = initial.clone();
        frame.hour += offset;
        // No output at a carried hour: that hour accrues no unnamed-holder evidence.
        if offset == 6 {
            *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable {
                reason: "held output unavailable".into(),
            };
        }
        if (27..35).contains(&offset)
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[1].tag = b"named again".to_vec();
        }
        frames.push(frame);
    }
    let mut fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(format.version(), 1, initial.hour);
    let mut tracker = FrameChangeTracker::new();
    let mut accepted = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        if let Some(page) = state
            .ingest_changed(
                format,
                frame,
                Named::default(),
                &flag,
                &tracker.observe(frame, format),
            )
            .unwrap()
        {
            publish_endpoint(&mut writer, &mut state, &page).unwrap();
        }
        accepted.push(frame.clone());
        if [11, 28, frames.len() - 1].contains(&index) {
            if let Some(page) = state.flush() {
                publish_endpoint(&mut writer, &mut state, &page).unwrap();
            }
            write_state(&mut writer, &mut state).unwrap();
            writer.commit().unwrap();
            let count = endpoint_count(&fixture.store, &fixture.key, 1).unwrap();
            assert!(
                count < accepted.len() as u64 / 2,
                "warning clock emitted hourly pages"
            );
            for (end, at) in accepted.iter().enumerate() {
                let replay = replay::container::Replay::from_records(
                    crate::test_fixtures::CAMPAIGN.into(),
                    &[crate::test_fixtures::records(1, accepted[..=end].to_vec())],
                )
                .unwrap();
                let oracle = equipment::production::scan(&replay, 1, &flag).unwrap();
                let actual = read_at(&fixture.store, &fixture.key, 1, count, at.hour, &flag)
                    .unwrap()
                    .unwrap();
                observer_tests::assert_reply_parity(
                    &actual.warnings,
                    &crate::products::equipment_produced::recording_warnings(&oracle),
                )
                .unwrap();
            }
            assert!(
                read_at(
                    &fixture.store,
                    &fixture.key,
                    1,
                    count,
                    initial.hour + 17,
                    &flag
                )
                .unwrap()
                .is_none()
            );
            if index == frames.len() - 1 {
                break;
            }
            state = latest_state(&fixture.store, &fixture.key, 1)
                .unwrap()
                .unwrap();
            tracker.invalidate();
            writer = if index == 28 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
        }
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn production_observer_carried_hours_stay_flat_at_5000_countries() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let first = observer_frame(5000, true);
    let format = observer_format(&first);
    let flag = AtomicBool::new(false);
    observer_tests::assert_flat(
        observer_tests::scaled_work(1, |hours| {
            let mut state = ProductionAccumulator::new(format.version(), 1, first.hour);
            let mut tracker = FrameChangeTracker::new();
            state.ingest_changed(
                format,
                &first,
                Named::default(),
                &flag,
                &tracker.observe(&first, format),
            )?;
            assert_eq!(state.pending.as_ref().unwrap().countries.len(), 4999);
            observer_tests::unchanged_frames(
                state.work(),
                &first,
                hours as i32,
                format,
                |frame, changes| {
                    state.ingest_changed(format, frame, Named::default(), &flag, changes)?;
                    Ok(state.work())
                },
            )
        })
        .unwrap(),
    );
}

#[test]
fn production_spent_reopens_equal_amounts_after_an_unknown_interval() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let frame = observer_frame(1, false);
    let format = observer_format(&frame);
    let flag = AtomicBool::new(false);
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    state
        .ingest(format, &frame, Named::default(), &flag)
        .unwrap();
    for (hour, reason) in [
        (9, Some(ProductionSpendingUnknown::SpendingUnavailable)),
        (10, None),
    ] {
        let track = state.result.tags.get_mut("mod country 1").unwrap();
        track.last.as_mut().unwrap().1 = hour;
        track.cost_current.get_mut(&(1, false)).unwrap().reason = reason;
        state.observe_spent(hour).unwrap();
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    state
        .spent
        .flush(&mut writer, "production-spent-1", 0, &flag)
        .unwrap();
    writer.commit().unwrap();
    let key =
        state.spent_keys[&serde_json::to_string(&("mod country 1", false, false, 1u64)).unwrap()];
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "production-spent-1".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader.value_at(key, 8).unwrap().map(|run| run.value),
        Some(7)
    );
    assert!(reader.value_at(key, 9).unwrap().is_none());
    assert_eq!(
        reader.value_at(key, 10).unwrap().map(|run| run.value),
        Some(7)
    );
}

fn params() -> ProducedParams {
    ProducedParams {
        branch_id: "1".into(),
        tags: vec!["A".into()],
        top: None,
        measure: ProducedMeasure::Total,
        count: ProducedCount::Units,
        grouping: ProducedGrouping::Type,
        included: Vec::new(),
        refits: false,
    }
}
fn amount(variant: u64, units: i128, exact: bool, refit: bool) -> Amount {
    Amount {
        variant: variant.to_string(),
        refit,
        units: units.to_string(),
        units_exact: exact,
        spent: units.to_string(),
        spending_reason: None,
    }
}
fn source(read: i32, amounts: Vec<Amount>) -> SourceEndpoint {
    SourceEndpoint {
        slot: 1,
        country_id: 2,
        observation: Some(0),
        segment: 1,
        read_hour: read,
        units_known: true,
        spending_known: true,
        present: Some(false),
        campaign_spending_reason: None,
        amounts,
    }
}
fn page(read: i32, amounts: Vec<Amount>) -> EndpointPage {
    EndpointPage {
        schema: 1,
        branch_id: "1".into(),
        first_hour: read,
        last_hour: read + 23,
        equipment_read: Some(read),
        rocket_read: Some(read),
        equipment_origin: Some(0),
        equipment_restart: None,
        rocket_origin: Some(0),
        rocket_restart: None,
        countries: vec![CountryEndpoint {
            tag: "A".into(),
            equipment: Some(source(read, amounts)),
            rocket: Some(source(read, Vec::new())),
            rocket_joined: true,
        }],
        classifications: BTreeMap::from([
            (
                1,
                Classification {
                    reason: None,
                    key: "mod_one".into(),
                    ordinal: Some(0),
                    domain: Some(Domain::Land),
                    name: Some("".into()),
                    names: Vec::new(),
                    seen: [true; 4],
                },
            ),
            (
                2,
                Classification {
                    reason: None,
                    key: "mod_two".into(),
                    ordinal: Some(1),
                    domain: Some(Domain::Air),
                    name: None,
                    names: Vec::new(),
                    seen: [true; 4],
                },
            ),
            (
                3,
                Classification {
                    reason: None,
                    key: ":unresolved".into(),
                    ordinal: None,
                    domain: None,
                    name: None,
                    names: Vec::new(),
                    seen: [true; 4],
                },
            ),
        ]),
        warnings: Vec::new(),
        unnamed_before: 0,
        unnamed_per_hour: 0,
        unnamed_warning_index: 0,
    }
}

#[test]
fn arbitrary_subsets_keep_signed_wide_sums_ties_request_order_and_refits_at_5000_countries() {
    let mut page = page(0, Vec::new());
    page.countries = (0..5000u32)
        .map(|index| {
            let mut equipment = source(
                0,
                vec![
                    amount(1, 9_007_199_254_740_993, true, false),
                    amount(2, -(i128::from(index) + 1), true, false),
                    amount(1, 500, true, true),
                ],
            );
            equipment.slot = index;
            equipment.country_id = index;
            let mut rocket = source(0, Vec::new());
            rocket.slot = index;
            rocket.country_id = index;
            CountryEndpoint {
                rocket_joined: true,
                tag: format!("M{index:04}"),
                equipment: Some(equipment),
                rocket: Some(rocket),
            }
        })
        .collect();
    let mut params = params();
    params.tags.clear();
    params.top = Some(5000);
    params.included = vec!["mod_one".into(), "mod_one".into()];
    let result = select(
        &page,
        None,
        None,
        &params,
        4998,
        20,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.total_countries, 5000);
    assert_eq!(
        result
            .countries
            .iter()
            .map(|row| row.tag.as_str())
            .collect::<Vec<_>>(),
        ["M4998", "M4999"]
    );
    assert_eq!(
        result.countries[0].total.as_deref(),
        Some("9007199254740993")
    );
    params.included.push("mod_two".into());
    params.refits = true;
    let result = select(&page, None, None, &params, 0, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].tag, "M0000");
    assert_eq!(
        result.countries[0].total.as_deref(),
        Some("9007199254741492")
    );
    params.top = None;
    params.tags = vec!["M0002".into(), "M0000".into(), "M0002".into()];
    let result = select(&page, None, None, &params, 0, 10, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|row| row.tag.as_str())
            .collect::<Vec<_>>(),
        ["M0002", "M0000"]
    );
}

#[test]
fn unresolved_subsets_and_rocket_absence_preserve_independent_equipment_values() {
    let mut page = page(
        0,
        vec![amount(1, 10, true, false), amount(3, -3, true, false)],
    );
    let mut params = params();
    let result = select(&page, None, None, &params, 0, 10, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].total.as_deref(), Some("7"));
    assert_eq!(result.countries[0].values[0].amount, None);
    params.included = vec!["mod_one".into()];
    let result = select(&page, None, None, &params, 0, 10, &AtomicBool::new(false)).unwrap();
    assert!(!result.known);
    assert_eq!(result.countries[0].total, None);
    params.included = vec![":unresolved".into()];
    page.countries[0].rocket = None;
    let result = select(&page, None, None, &params, 0, 10, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].total, None);
    assert_eq!(result.countries[0].equipment_total.as_deref(), Some("-3"));
    assert_eq!(
        result.countries[0].values[0].equipment_amount.as_deref(),
        Some("-3")
    );
}

#[test]
fn periods_use_exact_read_predecessors_and_never_bridge_holder_or_coverage_breaks() {
    let before = page(0, vec![amount(1, -4, true, false)]);
    let mut current = page(24, vec![amount(1, 7, true, false)]);
    let mut params = params();
    params.measure = ProducedMeasure::Day;
    let result = select(
        &current,
        Some(&before),
        Some(&before),
        &params,
        0,
        10,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.countries[0].total.as_deref(), Some("11"));
    assert_eq!(
        predecessor(current.equipment_read.unwrap(), params.measure),
        Some(0)
    );
    current.countries[0].equipment.as_mut().unwrap().segment += 1;
    assert!(
        !select(
            &current,
            Some(&before),
            Some(&before),
            &params,
            0,
            10,
            &AtomicBool::new(false)
        )
        .unwrap()
        .known
    );
    current.countries[0].equipment.as_mut().unwrap().segment -= 1;
    current.countries[0].equipment.as_mut().unwrap().country_id += 1;
    assert_eq!(
        select(
            &current,
            Some(&before),
            Some(&before),
            &params,
            0,
            10,
            &AtomicBool::new(false)
        )
        .unwrap()
        .countries[0]
            .total,
        None
    );
}

#[test]
fn manual_rank_pages_sort_wide_scores_before_paging_and_keep_the_default_request_order() {
    let mut page = page(0, vec![amount(1, 9_007_199_254_740_992, true, false)]);
    let mut second = page.countries[0].clone();
    second.tag = "B".into();
    second.equipment.as_mut().unwrap().slot = 2;
    second.equipment.as_mut().unwrap().country_id = 3;
    second.rocket.as_mut().unwrap().slot = 2;
    second.rocket.as_mut().unwrap().country_id = 3;
    second.equipment.as_mut().unwrap().amounts[0].units = "9007199254740993".into();
    page.countries.push(second);
    let mut params = params();
    params.tags.push("B".into());
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        select(&page, None, None, &params, 0, 1, &cancelled)
            .unwrap()
            .countries[0]
            .tag,
        "A"
    );
    assert_eq!(
        select_ordered(&page, None, None, &params, 0, 1, true, &cancelled)
            .unwrap()
            .countries[0]
            .tag,
        "B"
    );
    assert!(!compatible(
        &source(i32::MIN, Vec::new()),
        Some(&source(i32::MIN, Vec::new())),
        ProducedMeasure::Day
    ));
}

#[test]
fn cancelled_queries_refuse_before_returning_partial_selection() {
    assert_eq!(
        select(
            &page(0, Vec::new()),
            None,
            None,
            &params(),
            0,
            1,
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn persisted_pages_and_resume_states_isolate_overlapping_branch_hours() {
    let campaign = uuid::Uuid::new_v4();
    let scratch = std::env::temp_dir().join(format!("prepared-production-{campaign}"));
    std::fs::create_dir_all(&scratch).unwrap();
    let recording = scratch.join("synthetic-recording");
    std::fs::write(&recording, b"synthetic source metadata").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(campaign).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    let first = page(0, vec![amount(1, 7, true, false)]);
    let mut second = page(0, vec![amount(1, 9, true, false)]);
    second.branch_id = "2".into();
    let mut resumed = ProductionAccumulator::new(replay::FORMAT_VERSION, 1, 0);
    let mut other_branch = ProductionAccumulator::new(replay::FORMAT_VERSION, 2, 0);
    assert_eq!(
        publish_endpoint(&mut writer, &mut resumed, &first).unwrap(),
        0
    );
    assert_eq!(
        publish_endpoint(&mut writer, &mut other_branch, &second).unwrap(),
        0
    );
    write_state(&mut writer, &mut resumed).unwrap();
    let mut definition = first.classifications[&1].clone();
    definition.key = "mod_final".into();
    definition.names = vec![StockVariantName {
        first_hour: 20,
        name: Some("Later".into()),
        known: true,
        design: None,
    }];
    resumed.classifications.insert(1, definition);
    write_state(&mut writer, &mut resumed).unwrap();
    write_state(&mut writer, &mut other_branch).unwrap();
    writer.commit().unwrap();
    let cancelled = AtomicBool::new(false);
    let mut params = params();
    params.included = vec!["mod_final".into()];
    assert_eq!(
        select_at(&store, &key, 12, &params, 0, 10, &cancelled)
            .unwrap()
            .unwrap()
            .countries[0]
            .total
            .as_deref(),
        Some("7")
    );
    let mut types_only = params.clone();
    types_only.tags.clear();
    let types = select_at(&store, &key, 999, &types_only, 0, 0, &cancelled)
        .unwrap()
        .unwrap();
    assert!(types.countries.is_empty());
    assert!(types.keys.is_empty());
    assert!(types.types.iter().any(|kind| kind.key == "mod_final"));
    params.branch_id = "2".into();
    params.included.clear();
    assert_eq!(
        select_at(&store, &key, 12, &params, 0, 10, &cancelled)
            .unwrap()
            .unwrap()
            .countries[0]
            .total
            .as_deref(),
        Some("9")
    );
    assert_eq!(latest_state(&store, &key, 1).unwrap().unwrap().branch, 1);
    assert_eq!(latest_state(&store, &key, 2).unwrap().unwrap().branch, 2);
    assert_eq!(
        store.range_count(&key, &section(STATE, 1)).unwrap(),
        Some(1)
    );
    assert!(
        read_at(&store, &key, 3, 1, 12, &cancelled)
            .unwrap()
            .is_none()
    );
    let mut incompatible = first;
    incompatible.schema = 2;
    assert!(select(&incompatible, None, None, &params, 0, 10, &cancelled).is_err());
    store.clear().unwrap();
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn endpoint_leaf_events_restore_identical_pages_after_two_commits_resume_append_and_retirement() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay::FORMAT_VERSION, 1, 0);
    let wide = (1i128 << 100) + 17;
    let mut expected = Vec::new();
    for block in 0..5 {
        let read = block * 24;
        let mut current = page(
            read,
            vec![amount(1, wide, true, false), amount(2, -31, false, true)],
        );
        current.unnamed_before = block as usize * 24;
        current.unnamed_per_hour = 1;
        current.countries[0].rocket.as_mut().unwrap().amounts =
            vec![amount(0, -wide + i128::from(block), true, false)];
        if block >= 1 {
            current.classifications.get_mut(&1).unwrap().names = vec![StockVariantName {
                first_hour: 24,
                name: Some("late mod name".into()),
                known: true,
                design: None,
            }];
            current.countries[0].equipment.as_mut().unwrap().amounts[1].spending_reason =
                Some(ProductionSpendingUnknown::LostOutput);
        }
        if block == 2 {
            current.countries[0].equipment = None;
            current.countries[0].rocket_joined = false;
        }
        if block >= 3 {
            let equipment = current.countries[0].equipment.as_mut().unwrap();
            equipment.slot = 7;
            equipment.country_id = 19;
            equipment.segment = 2;
            equipment.present = None;
            current.countries[0].rocket = None;
            current.countries[0].rocket_joined = false;
        }
        if block == 4 {
            current.countries[0]
                .equipment
                .as_mut()
                .unwrap()
                .amounts
                .pop();
        }
        state.classifications = current.classifications.clone();
        publish_endpoint(&mut writer, &mut state, &current).unwrap();
        expected.push(current);
        if [1, 3, 4].contains(&block) {
            write_state(&mut writer, &mut state).unwrap();
            writer.commit().unwrap();
            let count = endpoint_count(&fixture.store, &fixture.key, 1).unwrap();
            for original in &expected {
                for hour in [original.first_hour, original.last_hour] {
                    let mut wanted = original.clone();
                    wanted.render_warnings(hour);
                    assert_eq!(
                        read_at(&fixture.store, &fixture.key, 1, count, hour, &flag)
                            .unwrap()
                            .unwrap(),
                        wanted,
                        "restored page at {hour} after block {block}",
                    );
                }
            }
            if block == 4 {
                break;
            }
            state = latest_state(&fixture.store, &fixture.key, 1)
                .unwrap()
                .unwrap();
            writer = if block == 3 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
        }
    }
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "production-spent-1".into(),
        &flag,
    )
    .unwrap();
    let token = serde_json::to_string(&("A", false, false, 1u64)).unwrap();
    // One initial value, its absence, and its new holder. Later reads carry it.
    assert_eq!(
        reader
            .event_count_through(state.spent_keys[&token], i32::MAX)
            .unwrap(),
        3
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn endpoint_written_bytes_are_flat_at_n_and_ten_n_blocks_and_retired_leaves_are_not_republished() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = ProductionAccumulator::new(replay::FORMAT_VERSION, 1, 0);
    let mut current = page(
        0,
        (1..=128)
            .map(|variant| amount(variant, i128::from(variant), true, false))
            .collect(),
    );
    let mut writer = fixture.writer();
    publish_endpoint(&mut writer, &mut state, &current).unwrap();
    write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    let mut measured = Vec::new();
    for block in 1..=40 {
        state = latest_state(&fixture.store, &fixture.key, 1)
            .unwrap()
            .unwrap();
        let mut writer = if block == 21 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        current.first_hour = block * 720;
        current.last_hour = current.first_hour + 719;
        current.equipment_read = Some(current.first_hour);
        current.countries[0].equipment.as_mut().unwrap().read_hour = current.first_hour;
        current.countries[0].equipment.as_mut().unwrap().amounts[0].units =
            (10_000 + block).to_string();
        current.countries[0].equipment.as_mut().unwrap().amounts[0].spent =
            (20_000 + block).to_string();
        publish_endpoint(&mut writer, &mut state, &current).unwrap();
        write_catalogue(&mut writer, &mut state, &flag).unwrap();
        let bytes = writer.bytes_written();
        write_state(&mut writer, &mut state).unwrap();
        writer.commit().unwrap();
        if [4, 40].contains(&block) {
            measured.push(bytes);
            let mut reader = Reader::new(
                &fixture.store,
                &fixture.key,
                "production-spent-1".into(),
                &flag,
            )
            .unwrap();
            for variant in 2..=128u64 {
                let token = serde_json::to_string(&("A", false, false, variant)).unwrap();
                assert_eq!(
                    reader
                        .event_count_through(state.spent_keys[&token], i32::MAX)
                        .unwrap(),
                    1,
                    "retired leaf {variant} was republished"
                );
            }
            let header: EndpointPage = crate::prepared::series::read_reply(
                &fixture.store,
                &fixture.key,
                &section(ENDPOINTS, 1),
                block as u64,
            )
            .unwrap()
            .unwrap();
            assert!(header.classifications.is_empty());
            assert!(header.countries.iter().all(|row| {
                [&row.equipment, &row.rocket]
                    .into_iter()
                    .flatten()
                    .all(|source| source.amounts.is_empty())
            }));
        }
    }
    // Identical activity and inventory: only integer encodings/date compression vary.
    assert!(
        measured[1].abs_diff(measured[0]) <= measured[0] / 100,
        "N/10N publication bytes grew: {measured:?}"
    );
    eprintln!(
        "production endpoint constant activity: N/10N block bytes {measured:?}; {} shared directory bytes per block",
        64 * (state.spent_keys.len() + state.classification_keys.len())
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn endpoint_changed_payload_stays_flat_when_retired_leaf_history_grows_tenfold() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut state = ProductionAccumulator::new(replay::FORMAT_VERSION, 1, 0);
    let mut current = page(0, Vec::new());
    current.classifications.clear();
    let mut measured = Vec::new();
    // Each block introduces one native variant, then retains its unchanged total.
    // The shared format still writes its directory and empty per-key blob; those
    // are index overhead, not another copy of a retired amount or classification.
    let empty_blob_bytes = crate::prepared::series::compress(&[0; 8], 3).unwrap().len() as u64;
    for block in 1..=40u64 {
        current.first_hour = block as i32 * 720;
        current.last_hour = current.first_hour + 719;
        current.equipment_read = Some(current.first_hour);
        let equipment = current.countries[0].equipment.as_mut().unwrap();
        equipment.read_hour = current.first_hour;
        equipment.amounts.push(amount(block, 17, true, false));
        let mut kind = page(0, Vec::new()).classifications.remove(&1).unwrap();
        kind.key = format!("mod variant {block:04}");
        current.classifications.insert(block, kind);
        let mut writer = fixture.writer();
        publish_endpoint(&mut writer, &mut state, &current).unwrap();
        state
            .spent
            .flush(&mut writer, "production-spent-1", 0, &flag)
            .unwrap();
        let written = writer.bytes_written();
        let keys = (state.spent_keys.len() + state.classification_keys.len()) as u64;
        let payload = written - keys * (64 + empty_blob_bytes);
        writer.commit().unwrap();
        if [4, 40].contains(&block) {
            measured.push(payload);
            let mut reader = Reader::new(
                &fixture.store,
                &fixture.key,
                "production-spent-1".into(),
                &flag,
            )
            .unwrap();
            for variant in 1..=block {
                let token = serde_json::to_string(&("A", false, false, variant)).unwrap();
                assert_eq!(
                    reader
                        .event_count_through(state.spent_keys[&token], i32::MAX)
                        .unwrap(),
                    1
                );
                assert_eq!(
                    reader
                        .event_count_through(state.classification_keys[&variant], i32::MAX)
                        .unwrap(),
                    1
                );
            }
        }
    }
    assert!(
        measured[1].abs_diff(measured[0]) <= 64,
        "changed endpoint payload grew with retired history: {measured:?}"
    );
    eprintln!(
        "production endpoints with N/10N retired leaves: {measured:?} changed-payload bytes per 720 h, plus 64 directory bytes and {empty_blob_bytes} empty-blob bytes per known key"
    );
}

#[test]
fn null_equipment_design_keeps_units_and_unresolved_attribution() {
    let mut current = page(0, vec![amount(0, 7, true, false)]);
    let unresolved = current.classifications[&3].clone();
    current.classifications.insert(0, unresolved);
    let cancelled = AtomicBool::new(false);
    let mut request = params();
    let selected = select(&current, None, None, &request, 0, 1, &cancelled).unwrap();
    assert_eq!(selected.countries[0].total.as_deref(), Some("7"));
    request.included = vec!["mod_one".into()];
    let selected = select(&current, None, None, &request, 0, 1, &cancelled).unwrap();
    assert_eq!(selected.countries[0].total, None);
}

#[test]
fn branch_aliases_compare_numerically_and_echo_the_request() {
    let current = page(0, vec![amount(1, 7, true, false)]);
    for alias in ["01", "+1"] {
        let mut request = params();
        request.branch_id = alias.into();
        let selected = select(
            &current,
            None,
            None,
            &request,
            0,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(selected.branch_id, alias);
        assert_eq!(selected.countries[0].total.as_deref(), Some("7"));
    }
}

#[test]
fn global_value_extent_includes_off_page_signed_rows_exactly() {
    let mut current = page(0, vec![amount(1, 9007199254740993, true, false)]);
    let mut negative = current.countries[0].clone();
    negative.tag = "B".into();
    negative.equipment.as_mut().unwrap().amounts = vec![amount(1, -7, true, false)];
    current.countries.push(negative);
    let mut request = params();
    request.tags.push("B".into());
    let selected = select(
        &current,
        None,
        None,
        &request,
        1,
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(selected.offset, "1");
    assert_eq!(selected.value_min, "-7");
    assert_eq!(selected.value_max, "9007199254740993");
    assert_eq!(selected.countries.len(), 1);
    assert_eq!(selected.countries[0].tag, "B");
}

#[test]
fn endpoint_reads_search_each_leaf_directory_once_and_restore_only_requested_leaves() {
    use crate::prepared::series::{directory_tests::SMALL_READS, tests::Fixture};
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay::FORMAT_VERSION, 1, 0);
    let tags: Vec<String> = (0..1000).map(|index| format!("T{index:04}")).collect();
    let mut published = Vec::new();
    // Three batches; every leaf changes in each, the classifications never do.
    for block in 0..3 {
        let read = block * 720;
        let mut current = page(read, Vec::new());
        current.last_hour = read + 719;
        current.countries = tags
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                let units = i128::from(block) * 1000 + index as i128;
                CountryEndpoint {
                    tag: tag.clone(),
                    equipment: Some(source(
                        read,
                        vec![
                            amount(1, units, true, false),
                            amount(2, -units, true, false),
                        ],
                    )),
                    rocket: Some(source(read, vec![amount(3, units, true, false)])),
                    rocket_joined: true,
                }
            })
            .collect();
        state.classifications = current.classifications.clone();
        publish_endpoint(&mut writer, &mut state, &current).unwrap();
        write_state(&mut writer, &mut state).unwrap();
        published.push(current);
    }
    writer.commit().unwrap();
    let count = endpoint_count(&fixture.store, &fixture.key, 1).unwrap();
    for (block, original) in published.iter().enumerate() {
        let hour = original.first_hour + 5;
        let mut wanted = original.clone();
        wanted.render_warnings(hour);
        let before = SMALL_READS.get();
        let restored = read_at(&fixture.store, &fixture.key, 1, count, hour, &flag)
            .unwrap()
            .unwrap();
        let reads = SMALL_READS.get() - before;
        assert_eq!(restored, wanted, "block {block}");
        // Searching each of the 3,003 leaves' directory entries on its own
        // takes about 14 small reads per leaf (42,000).
        assert!(reads <= 1_500, "{reads} small reads restoring every leaf");

        let mut endpoints = Endpoints::new(&fixture.store, &fixture.key, 1, count, &flag);
        let tag = tags[517].as_str();
        let before = SMALL_READS.get();
        let one = endpoints.page(hour, Leaves::Country(tag)).unwrap().unwrap();
        let reads = SMALL_READS.get() - before;
        assert!(reads <= 400, "{reads} small reads restoring one country");
        assert_eq!(one.classifications, restored.classifications);
        for (row, full) in one.countries.iter().zip(&restored.countries) {
            if row.tag == tag {
                assert_eq!(row, full);
            } else {
                assert!(row.equipment.as_ref().unwrap().amounts.is_empty());
                assert!(row.rocket.as_ref().unwrap().amounts.is_empty());
            }
        }
        let joined = BTreeSet::from([tags[3].clone(), tags[999].clone()]);
        let rockets = endpoints
            .page(hour, Leaves::Rockets(&joined))
            .unwrap()
            .unwrap();
        assert!(rockets.classifications.is_empty());
        for (row, full) in rockets.countries.iter().zip(&restored.countries) {
            assert!(row.equipment.as_ref().unwrap().amounts.is_empty());
            if joined.contains(&row.tag) {
                assert_eq!(row.rocket, full.rocket);
            } else {
                assert!(row.rocket.as_ref().unwrap().amounts.is_empty());
            }
        }
    }
}
