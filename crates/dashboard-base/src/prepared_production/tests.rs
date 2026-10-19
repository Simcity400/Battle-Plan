use super::*;

fn observer_frame(countries: u32, invalid_tag: bool) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::ProductionLineOutputRow,
        rows::ProductionLineRow, rows::RocketProductionOutputRow, rows::TableRows,
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

fn naming_frame() -> DecodedFrame {
    use replay_data::{
        rows::LexicalTokenRow, rows::ProductionLineOutputRow, rows::TableRows,
        rows::VariantDefinitionRow,
    };
    let mut frame = observer_frame(2, false);
    frame.hour = crate::test_fixtures::slot_hour(TableId::VariantDefinitions);
    frame.metrics.table_ns.fill(0);
    frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [(1, "mod_equipment_a"), (2, "mod_equipment_b")]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            (1..=2)
                .map(|id| VariantDefinitionRow {
                    id,
                    equipment_token: 1,
                    creator_link: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(
            (1..=2)
                .map(|variant| ProductionLineOutputRow {
                    line: 1,
                    ordinal: variant as u32,
                    country: 1,
                    variant,
                    units: 1,
                    since_hour: frame.hour,
                    exact: true,
                    spent_raw: Recorded::Value(Some(7)),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

#[test]
fn equal_variant_reads_build_and_classify_once_but_keep_each_reads_name_date() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let first = naming_frame();
    let format = observer_format(&first);
    let mut state = ProductionAccumulator::new(format.version(), 1, first.hour);
    let mut tracker = FrameChangeTracker::new();
    equipment::VARIANT_BUILDS.with(|count| count.set(0));
    for offset in 0..=72 {
        let mut frame = first.clone();
        frame.hour += offset;
        if offset != 0 {
            frame.metrics.table_ns.fill(0);
        }
        state
            .ingest_changed(
                format,
                &frame,
                Named::default(),
                &flag,
                &tracker.observe(&frame, format),
            )
            .unwrap();
        if offset % 24 == 0 {
            assert_eq!(state.classifications[&1].names[0].first_hour, frame.hour);
        }
    }
    assert_eq!(equipment::VARIANT_BUILDS.with(std::cell::Cell::get), 2);
    assert_eq!(state.variants.classifications, 2);
    let mut frame = first.clone();
    frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    for (offset, expected) in [(73, 3), (74, 4), (75, 5), (76, 7)] {
        use replay_data::{rows::TableRows, rows::VariantModuleRow, rows::VariantUpgradeRow};
        frame.hour = first.hour + offset;
        match offset {
            73 => {
                if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                    &mut *frame.tables[TableId::VariantDefinitions.index()]
                {
                    rows[0].display_name = b"mod design".to_vec();
                }
            }
            74 => {
                *frame.tables[TableId::VariantModules.index()] =
                    TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
                        variant: 1,
                        slot: 1,
                        definition_token: 2,
                        definition_active: 1,
                        ..Default::default()
                    }]))
            }
            75 => {
                *frame.tables[TableId::VariantUpgrades.index()] =
                    TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                        variant: 1,
                        definition_token: 2,
                        level: 3,
                        ..Default::default()
                    }]))
            }
            76 => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"mod renamed country".to_vec();
                }
            }
            _ => unreachable!(),
        }
        state
            .ingest_changed(
                format,
                &frame,
                Named::default(),
                &flag,
                &tracker.observe(&frame, format),
            )
            .unwrap();
        assert_eq!(
            equipment::VARIANT_BUILDS.with(std::cell::Cell::get),
            expected
        );
        assert_eq!(state.variants.classifications, expected);
    }
}

#[test]
fn variant_mutation_between_daily_slots_updates_ordinary_and_prepared_names() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::{TableRows, VariantChangeRow};
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let format = observer_format(&frame);
    let mut ordinary = equipment::VariantCatalogue::default();
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    let mut tracker = FrameChangeTracker::new();
    ordinary.observe(format, &frame);
    state
        .ingest_changed(
            format,
            &frame,
            Named::default(),
            &flag,
            &tracker.observe(&frame, format),
        )
        .unwrap();
    frame.hour += 1;
    frame.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *frame.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].display_name = b"between samples".to_vec();
    }
    *frame.tables[TableId::VariantChanges.index()] =
        TableView::Complete(TableRows::VariantChanges(vec![VariantChangeRow {
            sequence: 1,
            hour: frame.hour,
            variant: 1,
            flags: Some([1, 1, 1]),
            ..Default::default()
        }]));
    ordinary.observe(format, &frame);
    state
        .ingest_changed(
            format,
            &frame,
            Named::default(),
            &flag,
            &tracker.observe(&frame, format),
        )
        .unwrap();
    let ordinary = ordinary.versions[&1].last().unwrap();
    let prepared = state.variants.catalogue.versions[&1].last().unwrap();
    assert_eq!(ordinary.first_hour, frame.hour);
    assert_eq!(ordinary, prepared);
    assert_eq!(
        ordinary.raw.as_ref().unwrap().display_name,
        "between samples"
    );
}

#[test]
fn endpoint_events_serialize_once_for_hashing_and_publication() {
    struct Payload(std::cell::Cell<usize>);
    impl Serialize for Payload {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            serializer.serialize_str("mod payload")
        }
    }
    let mut state = ProductionAccumulator::new(FormatVersion::CURRENT.version(), 1, 8);
    let payload = Payload(std::cell::Cell::new(0));
    state.endpoint_event(0, 8, &payload).unwrap();
    state.endpoint_event(0, 9, &payload).unwrap();
    assert_eq!(payload.0.get(), 2);
    assert_eq!(state.spent.state(0).unwrap().events, 1);
}

#[test]
fn cached_variant_operands_keep_fresh_frame_pages_state_and_publication_across_resume() {
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        series::tests::{Fixture, assert_payloads_equal},
    };
    use replay_data::{rows::TableRows, rows::VariantModuleRow, rows::VariantUpgradeRow};
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let first = frame.hour;
    let format = observer_format(&frame);
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut held = [HeldPages::default(), HeldPages::default()];
    let mut states = [
        ProductionAccumulator::new(format.version(), 1, first),
        ProductionAccumulator::new(format.version(), 1, first),
    ];
    let mut tracker = FrameChangeTracker::new();
    let mut definitions = None;
    for offset in 0..=120 {
        if offset == 73 {
            continue;
        }
        frame.hour = first + offset;
        if offset != 0 {
            frame.metrics.table_ns.fill(0);
        }
        match offset {
            24 => {
                *frame.tables[TableId::VariantModules.index()] =
                    TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
                        variant: 1,
                        slot: 1,
                        definition_token: 2,
                        definition_active: 1,
                        ..Default::default()
                    }]))
            }
            32 => {
                if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                    &mut *frame.tables[TableId::VariantDefinitions.index()]
                {
                    rows[1].display_name = b"mod renamed design".to_vec();
                }
            }
            48 => {
                *frame.tables[TableId::VariantUpgrades.index()] =
                    TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                        variant: 1,
                        definition_token: 2,
                        level: 3,
                        ..Default::default()
                    }]))
            }
            50 => {
                definitions = Some(frame.tables[TableId::VariantDefinitions.index()].clone());
                *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
                    reason: "synthetic unread names".into(),
                };
            }
            54 => {
                frame.tables[TableId::VariantDefinitions.index()] = definitions.take().unwrap();
                frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
            }
            56 => {
                if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                    &mut *frame.tables[TableId::VariantDefinitions.index()]
                {
                    rows.retain(|row| row.id != 2);
                }
            }
            64 => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"renamed mod country".to_vec();
                }
            }
            80 | 104 => {
                if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                    &mut *frame.tables[TableId::LexicalTokens.index()]
                {
                    rows[0].literal = if offset == 80 {
                        vec![255]
                    } else {
                        b"mod_equipment_a".to_vec()
                    };
                }
            }
            _ => {}
        }
        let changes = tracker.observe(&frame, format);
        let mut pages = Vec::new();
        for (index, state) in states.iter_mut().enumerate() {
            if index == 1 {
                state.variants = ProductionVariants::default();
            }
            pages.push(
                state
                    .ingest_changed(format, &frame, Named::default(), &flag, &changes)
                    .unwrap(),
            );
        }
        assert_eq!(
            serde_json::to_vec(&pages[0]).unwrap(),
            serde_json::to_vec(&pages[1]).unwrap(),
            "completed page at {offset}"
        );
        for (index, ((state, fixture), page)) in
            states.iter_mut().zip(&fixtures).zip(pages).enumerate()
        {
            if let Some(page) = page {
                publish_endpoint_to(&mut held[index], state, &page).unwrap();
            }
            if [36, 72, 120].contains(&offset) {
                let mut writer = fixture.writer();
                held[index].write(&mut writer).unwrap();
                if let Some(page) = state.flush() {
                    publish_endpoint(&mut writer, state, &page).unwrap();
                }
                write_state(&mut writer, state).unwrap();
                writer.commit().unwrap();
            }
        }
        assert_eq!(
            serde_json::to_vec(&states[0]).unwrap(),
            serde_json::to_vec(&states[1]).unwrap(),
            "state at {offset}"
        );
        if [36, 72, 120].contains(&offset) {
            assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "cached and fresh naming",
            );
            for (state, fixture) in states.iter_mut().zip(&fixtures) {
                *state = latest_state(&fixture.store, &fixture.key, 1)
                    .unwrap()
                    .unwrap();
            }
            tracker.invalidate();
        }
    }
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
                if let TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
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
            if let TableView::Complete(replay_data::rows::TableRows::ProductionLineOutput(rows)) =
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

#[test]
fn production_endpoint_warning_clock_matches_every_oracle_hour_across_two_commits_resume_append_and_gaps()
 {
    use crate::prepared::series::tests::Fixture;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay_data::rows::TableRows;
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
        // The production observer versions each change to the cost inventory.
        track.cost_taken.as_mut().unwrap().1 += 1;
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
fn automatic_type_totals_include_off_page_countries_and_keep_type_order() {
    let mut current = page(
        0,
        vec![amount(1, 10, true, false), amount(2, 7, true, false)],
    );
    let mut second = current.countries[0].clone();
    second.tag = "B".into();
    second.equipment.as_mut().unwrap().amounts =
        vec![amount(1, 2, true, false), amount(2, 20, true, false)];
    current.countries.push(second);
    let mut input = params();
    input.tags.push("B".into());
    let selected = select(&current, None, None, &input, 0, 0, &AtomicBool::new(false)).unwrap();
    assert!(selected.countries.is_empty());
    assert_eq!(
        selected.type_totals,
        Some(vec![
            ("mod_one".into(), "12".into()),
            ("mod_two".into(), "27".into()),
            (":unresolved".into(), "0".into())
        ])
    );
}

#[test]
fn an_open_rocket_join_can_change_an_earlier_committed_endpoint_only_under_an_approximate_prefix() {
    use crate::prepared::{
        coverage::{Coverage, SCHEMA},
        prefix::Prefix,
    };
    let flag = AtomicBool::new(false);
    let replay = replay::container::Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            (0..=9)
                .map(|hour| crate::test_fixtures::source(1, hour, Some(1)))
                .collect(),
        )],
    )
    .unwrap();
    let (recording, root) = crate::test_fixtures::file_source(replay);
    let (store, key) = crate::prepared::files(&recording, None).unwrap();
    let mut state = ProductionAccumulator::new(1, 1, 0);
    let mut early = page(0, vec![amount(1, 4, true, false)]);
    early.last_hour = 7;
    let mut open = page(8, vec![amount(1, 5, true, false)]);
    open.last_hour = 8;
    state.classifications = open.classifications.clone();
    let eq = open.countries[0].equipment.as_ref().unwrap();
    assert!(
        state
            .joins
            .bind_rocket("A", eq, open.countries[0].rocket.as_ref(), 8, &mut None)
    );
    let mut writer = store.replace(key.clone()).unwrap();
    publish_endpoint(&mut writer, &mut state, &early).unwrap();
    publish_endpoint(&mut writer, &mut state, &open).unwrap();
    write_catalogue(&mut writer, &mut state, &flag).unwrap();
    writer.commit().unwrap();
    let before = select_at(&store, &key, 8, &params(), 0, 1, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(before.countries[0].total.as_deref(), Some("5"));
    assert_eq!(before.keys[0].key, "mod_one");
    let prefix = Prefix::new(
        &recording,
        1,
        Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 8,
            complete: false,
        },
        Some(8),
    );
    let mut provisional = serde_json::to_value(before).unwrap();
    prefix
        .mark_revisable(&recording, 1, &mut provisional)
        .unwrap();
    assert!(provisional["pending"].as_array().unwrap().is_empty());
    assert_eq!(
        provisional["approximate"],
        serde_json::json!([{"firstHour": 0, "lastHour": 8}])
    );
    assert!(!state.joins.bind_rocket("A", eq, None, 9, &mut None));
    state.classifications.get_mut(&1).unwrap().key = "mod_reclassified".into();
    let mut writer = store.extend(&key).unwrap();
    write_catalogue(&mut writer, &mut state, &flag).unwrap();
    writer.commit().unwrap();
    let after = select_at(&store, &key, 8, &params(), 0, 1, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(
        after.countries[0].total, None,
        "the next commit repairs the open supplying interval"
    );
    assert_eq!(
        after.keys[0].key, "mod_reclassified",
        "later classification revises the same earlier hour"
    );
    let unaffected = select_at(&store, &key, 0, &params(), 0, 1, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(unaffected.countries[0].total.as_deref(), Some("4"));
    drop(recording);
    let _ = std::fs::remove_dir_all(root);
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
    let mut resumed = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    let mut other_branch = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 2, 0);
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
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
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

#[test]
fn endpoint_written_bytes_are_flat_at_n_and_ten_n_blocks_and_retired_leaves_are_not_republished() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
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

#[test]
fn endpoint_changed_payload_stays_flat_when_retired_leaf_history_grows_tenfold() {
    use crate::prepared::series::{Reader, tests::Fixture};
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
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
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
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

#[test]
fn production_spending_projects_equal_inventories_once_and_rebuilds_after_restore() {
    use crate::prepared::series::tests::{Fixture, assert_payloads_equal};
    use replay_data::{rows::ProductionLineOutputRow, rows::TableRows};
    let flag = AtomicBool::new(false);
    let mut frame = observer_frame(1, false);
    let format = observer_format(&frame);
    let mut outputs: Vec<_> = (0..128)
        .map(|ordinal| ProductionLineOutputRow {
            line: 1,
            country: 1,
            variant: u64::from(ordinal) + 1,
            ordinal,
            units: 1,
            since_hour: frame.hour,
            exact: true,
            spent_raw: Recorded::Value(Some(7)),
            ..Default::default()
        })
        .collect();
    let mut states = [
        ProductionAccumulator::new(format.version(), 1, frame.hour),
        ProductionAccumulator::new(format.version(), 1, frame.hour),
    ];
    let fixtures = [Fixture::new(), Fixture::new()];
    for offset in 0..10 {
        frame.hour = 8 + offset + i32::from(offset >= 8);
        frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
        if offset == 6
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"replacement holder".to_vec();
        }
        if offset == 3 {
            outputs[0].spent_raw = Recorded::Value(None);
        }
        if offset == 4 {
            outputs[0].spent_raw = Recorded::Value(Some(7));
        }
        if offset == 5 {
            outputs[0].units = 2;
        }
        frame.tables[TableId::ProductionLineOutput.index()] =
            TableView::Complete(TableRows::ProductionLineOutput(outputs.clone())).into();
        if offset == 7 {
            frame.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable {
                reason: "synthetic output refusal".into(),
            }
            .into();
        }
        for (index, state) in states.iter_mut().enumerate() {
            // Rebuilding the runtime projection at every hour is the full
            // inventory reference, including every formerly active key.
            if index == 1 {
                state.spent_holders = Default::default();
                state.spent_holders_ready = false;
            }
            let before = state.spent_visits;
            state
                .ingest(format, &frame, Named::default(), &flag)
                .unwrap();
            if offset == 1 || offset == 2 {
                assert_eq!(
                    state.spent_visits - before,
                    if index == 0 { 0 } else { 256 }
                );
            }
        }
        assert_eq!(
            serde_json::to_vec(&states[0]).unwrap(),
            serde_json::to_vec(&states[1]).unwrap()
        );
        if offset == 2 || offset == 9 {
            for (state, fixture) in states.iter_mut().zip(&fixtures) {
                let mut writer = fixture.writer();
                state
                    .spent
                    .flush(&mut writer, "spending", 0, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "spending projection",
            );
        }
        if offset == 2 {
            for state in &mut states {
                *state = serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap();
            }
        }
    }
}

#[test]
fn spending_inventory_visits_follow_changed_holders_across_quiet_spans_and_hourly_carries() {
    use crate::prepared::{
        frame_changes::FrameChanges,
        series::tests::{Fixture, assert_payloads_equal},
    };
    use replay_data::{rows::ProductionLineOutputRow, rows::ProductionLineRow, rows::TableRows};
    const VARIANTS: u32 = 32;
    let flag = AtomicBool::new(false);
    for quiet in [true, false] {
        let mut frame = observer_frame(2, false);
        frame.tables[TableId::ProductionLines.index()] =
            TableView::Complete(TableRows::ProductionLines(
                (1..=2)
                    .map(|country| ProductionLineRow {
                        handle: u64::from(country),
                        country,
                        variant: 1,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        let mut output: Vec<_> = (1..=2)
            .flat_map(|country| {
                (0..VARIANTS).map(move |ordinal| ProductionLineOutputRow {
                    line: u64::from(country),
                    country,
                    ordinal,
                    variant: u64::from(ordinal) + 1,
                    units: 1,
                    since_hour: 8,
                    exact: true,
                    spent_raw: Recorded::Value(Some(7)),
                    ..Default::default()
                })
            })
            .collect();
        let format = observer_format(&frame);
        let mut states = [
            ProductionAccumulator::new(format.version(), 1, 8),
            ProductionAccumulator::new(format.version(), 1, 8),
        ];
        for day in 0..VARIANTS {
            let hour = 8 + 24 * day as i32;
            if day > 0 {
                let first = hour - 23;
                if quiet {
                    for state in &mut states {
                        assert!(state.advance_quiet(first, hour - 1).unwrap());
                    }
                } else {
                    let changes = FrameChanges {
                        rows_changed: [false; replay_data::rows::TABLE_COUNT],
                        table_read: [false; replay_data::rows::TABLE_COUNT],
                        slot_read: [false; replay_data::rows::TABLE_COUNT],
                        updates: None,
                    };
                    frame.metrics.table_ns.fill(0);
                    for carried in first..hour {
                        frame.hour = carried;
                        for state in &mut states {
                            assert!(
                                state
                                    .ingest_changed(
                                        format,
                                        &frame,
                                        Named::default(),
                                        &flag,
                                        &changes
                                    )
                                    .unwrap()
                                    .is_none()
                            );
                        }
                    }
                }
            }
            // One owner's cost changes once; the other inventory stays equal.
            if day == VARIANTS / 2 {
                output[0].spent_raw = Recorded::Value(Some(9));
            }
            frame.hour = hour;
            frame.metrics.table_ns.fill(0);
            frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
            frame.tables[TableId::ProductionLineOutput.index()] =
                TableView::Complete(TableRows::ProductionLineOutput(output.clone())).into();
            let mut changes = FrameChanges {
                rows_changed: [day == 0; replay_data::rows::TABLE_COUNT],
                table_read: [false; replay_data::rows::TABLE_COUNT],
                slot_read: [false; replay_data::rows::TABLE_COUNT],
                updates: None,
            };
            changes.rows_changed[TableId::ProductionLineOutput.index()] = true;
            changes.table_read[TableId::ProductionLineOutput.index()] = true;
            changes.slot_read[TableId::ProductionLineOutput.index()] = true;
            for (index, state) in states.iter_mut().enumerate() {
                if index == 1 {
                    state.spent_holders = Default::default();
                    state.spent_holders_ready = false;
                }
                let before = state.spent_visits;
                state
                    .ingest_changed(format, &frame, Named::default(), &flag, &changes)
                    .unwrap();
                let owners = if index == 1 || day == 0 {
                    2
                } else {
                    u32::from(day == VARIANTS / 2)
                };
                assert_eq!(
                    state.spent_visits - before,
                    (2 * VARIANTS * owners) as usize,
                    "quiet={quiet}, day={day}: only changed holders project"
                );
            }
            assert_eq!(
                serde_json::to_vec(&states[0]).unwrap(),
                serde_json::to_vec(&states[1]).unwrap()
            );
        }
        assert_eq!(states[0].spent_visits, (6 * VARIANTS) as usize);
        let fixtures = [Fixture::new(), Fixture::new()];
        for (state, fixture) in states.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            state
                .spent
                .flush(&mut writer, "spending", 0, &flag)
                .unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "quiet spending",
        );
    }
}

fn presentation_inventory_work_follows_changed_holders(count: u32) {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::ProductionLineOutputRow, rows::ProductionLineRow, rows::TableRows};
    let flag = AtomicBool::new(false);
    let mut frame = observer_frame(count, false);
    let first = frame.hour;
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            (1..=count)
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
            (1..=count)
                .map(|id| ProductionLineOutputRow {
                    line: u64::from(id),
                    country: id,
                    variant: 1,
                    units: 1,
                    since_hour: first,
                    exact: true,
                    spent_raw: Recorded::Value(Some(7)),
                    ..Default::default()
                })
                .collect(),
        ));
    let format = observer_format(&frame);
    let mut state = ProductionAccumulator::new(format.version(), 1, first);
    let mut tracker = FrameChangeTracker::new();
    frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
    state
        .ingest_changed(
            format,
            &frame,
            Named::default(),
            &flag,
            &tracker.observe(&frame, format),
        )
        .unwrap();
    assert_eq!(state.variants.inventory_visits, 2 * count as usize);
    for offset in 1..=4 {
        frame.hour = first + offset;
        frame.metrics.table_ns.fill(0);
        // A real presentation read with unchanged numeric inventories.
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
        let before = state.variants.inventory_visits;
        state
            .ingest_changed(
                format,
                &frame,
                Named::default(),
                &flag,
                &tracker.observe(&frame, format),
            )
            .unwrap();
        assert_eq!(
            state.variants.inventory_visits - before,
            0,
            "presentation at {count} holders must carry unchanged inventory keys"
        );
    }
    frame.hour += 1;
    frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
    if let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
        &mut *frame.tables[TableId::ProductionLineOutput.index()]
    {
        rows[0].units += 1;
    }
    let before = state.variants.inventory_visits;
    state
        .ingest_changed(
            format,
            &frame,
            Named::default(),
            &flag,
            &tracker.observe(&frame, format),
        )
        .unwrap();
    assert_eq!(
        state.variants.inventory_visits - before,
        2,
        "only the changed holder's two keys are visited"
    );
}

#[test]
fn presentation_inventory_carries_keys_and_replaces_only_the_changed_holder() {
    presentation_inventory_work_follows_changed_holders(8);
}

#[test]
fn presentation_inventory_carries_keys_at_5000_countries() {
    presentation_inventory_work_follows_changed_holders(5000);
}
