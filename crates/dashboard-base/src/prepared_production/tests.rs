use super::*;
use crate::prepared::series::tests::Fixture;
use crate::products::equipment::{Domain, StockVariantName};
use crate::products::equipment_produced::{ProducedCount, ProducedGrouping, ProducedParams};
use replay_data::rows::OutputPurpose;

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
                purpose: Recorded::Value(OutputPurpose::New.raw()),
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
                    purpose: Recorded::Value(OutputPurpose::New.raw()),
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
            .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
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
            .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
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
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
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
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let ordinary = ordinary.versions[&1].last().unwrap();
    let prepared = state.variants.catalogue.versions[&1].last().unwrap();
    assert_eq!(ordinary.first_hour, frame.hour);
    assert_eq!(ordinary, prepared);
    assert_eq!(
        ordinary.raw.as_ref().unwrap().display_name,
        "between samples"
    );
    // The classification names that read, as its raw operands, dated by it.
    let name = state.classifications[&1].names[0];
    assert_eq!(name.first_hour, frame.hour);
    assert_eq!(
        state.raw(name.raw.unwrap()).display_name,
        "between samples"
    );
}

fn unchanged_naming_witness(frame: &mut DecodedFrame) {
    use replay_data::rows::{TableRows, VariantChangeRow};
    frame.hour += 1;
    frame.metrics.table_ns.fill(0);
    *frame.tables[TableId::VariantChanges.index()] =
        TableView::Complete(TableRows::VariantChanges(vec![VariantChangeRow {
            sequence: frame.hour as u64,
            hour: frame.hour,
            variant: 1,
            flags: Some([1, 1, 1]),
            ..Default::default()
        }]));
}

#[test]
fn unchanged_naming_witness_matches_across_resume() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let format = observer_format(&frame);
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    let mut tracker = FrameChangeTracker::new();
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    let mut resumed = latest_state(&fixture.store, &fixture.key, 1)
        .unwrap()
        .unwrap();
    state.completed.clear();
    let names = state.classifications.clone();
    unchanged_naming_witness(&mut frame);
    // The resumed state holds no countries: both sides take the hour as a fresh observation.
    tracker.invalidate();
    let changes = tracker.observe(&frame, format);
    state
        .ingest_changed(format, &frame, &flag, &changes)
        .unwrap();
    resumed
        .ingest_changed(format, &frame, &flag, &changes)
        .unwrap();
    assert_eq!(state.classifications, names);
    assert_eq!(resumed.classifications, names);
    assert_eq!(
        state.pending.as_ref().unwrap().page,
        resumed.pending.as_ref().unwrap().page
    );
    assert_eq!(state.completed, resumed.completed);
}

#[test]
fn a_disappearing_holder_keeps_the_census_date_of_a_design_another_holder_still_selects() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::TableRows;
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let format = observer_format(&frame);
    if let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
        &mut *frame.tables[TableId::ProductionLineOutput.index()]
    {
        let mut second = rows[0].clone();
        second.line = 2;
        second.country = 2;
        rows.push(second);
    }
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    let mut tracker = FrameChangeTracker::new();
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let census = frame.hour;
    unchanged_naming_witness(&mut frame);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != 2);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != 2);
    }
    if let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
        &mut *frame.tables[TableId::ProductionLineOutput.index()]
    {
        rows.retain(|row| row.country != 2);
    }
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    // Country 2 no longer selects design 1, which country 1 still does: the ledger hour
    // reads no name, so both keep the census's date.
    assert_eq!(state.classifications[&1].names[0].first_hour, census);
    assert_eq!(state.classifications[&2].names[0].first_hour, census);
    assert!(state.variants.classified.contains(&1));
}

#[test]
fn a_selection_change_dates_names_at_the_read_they_came_from() {
    let name = |first_hour| VariantName {
        first_hour,
        raw: Some(1),
    };
    let redated = |own: i32, census: Option<i32>, at: i32| {
        let mut held = BTreeMap::from([(
            1,
            Classification {
                token: "mod_equipment_a".into(),
                names: vec![name(own)],
                seen: [false; SEEN],
            },
        )]);
        let (mut moved, mut changed) = (false, Vec::new());
        let mut classes = ClassEdits {
            held: &mut held,
            moved: &mut moved,
            changed: &mut changed,
            census,
            known: census,
        };
        classes.redate(1, at);
        (held[&1].names[0].first_hour, moved, changed)
    };
    // Unselected at the last census, which still read it: dated by that census.
    assert_eq!(redated(10, Some(34), 44), (34, true, vec![1]));
    // Named or renamed after the census: its own read stands, and nothing moves.
    assert_eq!(redated(40, Some(34), 44), (40, false, Vec::new()));
    assert_eq!(redated(40, None, 44), (40, false, Vec::new()));
    // At a census both are that census.
    assert_eq!(redated(10, Some(44), 44), (44, true, vec![1]));
}

#[test]
fn unchanged_naming_witness_restores_forgotten_names_once() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let format = observer_format(&frame);
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    let mut tracker = FrameChangeTracker::new();
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let lexical = frame.tables[TableId::LexicalTokens.index()].clone();
    unchanged_naming_witness(&mut frame);
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic missing lexical data".into(),
    };
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    assert!(state.classifications[&1].names[0].raw.is_none());
    unchanged_naming_witness(&mut frame);
    frame.tables[TableId::LexicalTokens.index()] = lexical;
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let restored = state.classifications.clone();
    assert!(restored[&1].names[0].raw.is_some());
    assert_eq!(restored[&1].names[0].first_hour, frame.hour);
    unchanged_naming_witness(&mut frame);
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    assert_eq!(state.classifications, restored);
}

#[test]
fn full_inventory_fallback_dates_only_membership_changes() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let flag = AtomicBool::new(false);
    let frame = naming_frame();
    let format = observer_format(&frame);
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    let mut tracker = FrameChangeTracker::new();
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    let names = state.classifications.clone();
    state.variants.changed.clear();
    let track = state.result.tags.values_mut().next().unwrap();
    for _ in 0..4097 {
        track.edits.note((1, Purpose::New));
    }
    track.taken.as_mut().unwrap().1 += 1;
    state.present(frame.hour + 1, true, false, true);
    assert_eq!(state.classifications, names);
    assert!(state.scratch.members.is_empty());
    let track = state.result.tags.values_mut().next().unwrap();
    let value = track.current.remove(&(1, Purpose::New)).unwrap();
    track.current.insert((1, Purpose::Unknown), value);
    track.taken.as_mut().unwrap().1 += 1;
    track.edits.replaced();
    state.present(frame.hour + 2, true, false, true);
    // The membership change reaches design 1 only, and its names keep the census's date.
    assert_ne!(state.classifications[&1].seen, names[&1].seen);
    assert_eq!(state.classifications[&1].names, names[&1].names);
    assert_eq!(state.classifications[&2], names[&2]);
}

#[test]
fn cached_variant_operands_keep_fresh_frame_pages_state_and_publication_across_resume() {
    use crate::prepared::{frame_changes::FrameChangeTracker, series::tests::assert_payloads_equal};
    use replay_data::{rows::TableRows, rows::VariantModuleRow, rows::VariantUpgradeRow};
    let flag = AtomicBool::new(false);
    let mut frame = naming_frame();
    let first = frame.hour;
    let format = observer_format(&frame);
    let fixtures = [Fixture::new(), Fixture::new()];
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
        for (index, state) in states.iter_mut().enumerate() {
            if index == 1 {
                state.variants.reads = VariantReads::default();
            }
            state
                .ingest_changed(format, &frame, &flag, &changes)
                .unwrap();
        }
        assert_eq!(
            states[0].completed, states[1].completed,
            "completed pages at {offset}"
        );
        if [36, 72, 120].contains(&offset) {
            for (state, fixture) in states.iter_mut().zip(&fixtures) {
                let mut writer = fixture.writer();
                write_state(&mut writer, state).unwrap();
                writer.commit().unwrap();
            }
            assert_eq!(states[0].completed, states[1].completed);
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
            state.ingest_changed(format, &first, &flag, &tracker.observe(&first, format))?;
            observer_tests::unchanged_frames(
                state.work(),
                &first,
                hours as i32,
                format,
                |frame, changes| {
                    state.ingest_changed(format, frame, &flag, changes)?;
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
                state.ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))?;
            }
            let before = state.work();
            frame.hour += 1;
            if let TableView::Complete(replay_data::rows::TableRows::ProductionLineOutput(rows)) =
                &mut *frame.tables[TableId::ProductionLineOutput.index()]
            {
                rows[0].units = 2;
                rows[0].spent_raw = Recorded::Value(Some(9));
            }
            state.ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))?;
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

/// `page`'s warnings at `hour`, its unnamed country-hours counted then.
fn warnings_at(page: &EndpointPage, hour: i32) -> Vec<String> {
    let mut warnings = page.warnings.clone();
    if let Some(warning) =
        crate::products::equipment_produced::unnamed_recording_warning(page.unnamed_at(hour))
    {
        warnings.insert(page.unnamed_warning_index, warning);
    }
    warnings
}

#[test]
fn production_endpoint_warning_clock_matches_every_oracle_hour_across_two_commits_resume_append_and_gaps()
 {
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
    let mut pages: Vec<EndpointPage> = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        state
            .ingest_changed(format, frame, &flag, &tracker.observe(frame, format))
            .unwrap();
        pages.extend(state.completed.drain(..).map(|(page, _)| page));
        accepted.push(frame.clone());
        if [11, 28, frames.len() - 1].contains(&index) {
            write_state(&mut writer, &mut state).unwrap();
            pages.extend(state.completed.drain(..).map(|(page, _)| page));
            writer.commit().unwrap();
            assert!(
                pages.len() < accepted.len() / 2,
                "warning clock emitted hourly pages"
            );
            for (end, at) in accepted.iter().enumerate() {
                let replay = replay::container::Replay::from_records(
                    crate::test_fixtures::CAMPAIGN.into(),
                    &[crate::test_fixtures::records(1, accepted[..=end].to_vec())],
                )
                .unwrap();
                let oracle = equipment::production::scan(&replay, 1, &flag).unwrap();
                let page = pages
                    .iter()
                    .find(|page| page.first_hour <= at.hour && at.hour <= page.last_hour)
                    .expect("an endpoint at every accepted hour");
                observer_tests::assert_reply_parity(
                    &warnings_at(page, at.hour),
                    &crate::products::equipment_produced::recording_warnings(&oracle),
                )
                .unwrap();
            }
            let gap = initial.hour + 17;
            assert!(
                pages
                    .iter()
                    .all(|page| !(page.first_hour..=page.last_hour).contains(&gap))
            );
            // The publication's warnings are its last endpoint's at its last hour.
            let published = read::read_index(&fixture.store, &fixture.key, 1)
                .unwrap()
                .unwrap();
            assert_eq!(
                published.warnings(),
                warnings_at(pages.last().unwrap(), accepted.last().unwrap().hour)
            );
            assert_eq!(published.endpoints.len(), pages.len());
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
fn production_spent_reopens_equal_amounts_after_an_unknown_interval() {
    use crate::prepared::series::Reader;
    let frame = observer_frame(1, false);
    let format = observer_format(&frame);
    let flag = AtomicBool::new(false);
    let mut state = ProductionAccumulator::new(format.version(), 1, frame.hour);
    state.ingest(format, &frame, &flag).unwrap();
    for (hour, reason) in [
        (9, Some(ProductionSpendingUnknown::SpendingUnavailable)),
        (10, None),
    ] {
        let track = state.result.tags.get_mut("mod country 1").unwrap();
        track.last.as_mut().unwrap().1 = hour;
        track.cost_current.get_mut(&(1, Purpose::New)).unwrap().reason = reason;
        // The production observer versions each change to the cost inventory.
        track.cost_taken.as_mut().unwrap().1 += 1;
        state.observe_spent(hour, true).unwrap();
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    state
        .spent
        .flush(&mut writer, "production-spent-1", 0, &flag)
        .unwrap();
    writer.commit().unwrap();
    let key = state.spent_keys()[&("mod country 1".to_owned(), false, false, 1u64)];
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
fn amount(variant: u64, units: i128, exact: bool, purpose: Purpose) -> Amount {
    Amount {
        variant,
        purpose,
        units,
        units_exact: exact,
        spent: units,
        spending_reason: None,
        unproven: 0,
    }
}
fn source(read: i32, amounts: Vec<Amount>) -> SourceEndpoint {
    SourceEndpoint {
        spending_since: read,
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
fn classified(token: &str) -> Classification {
    Classification {
        token: token.into(),
        names: Vec::new(),
        seen: [true; SEEN],
    }
}
fn page(read: i32, amounts: Vec<Amount>) -> EndpointPage {
    EndpointPage {
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
            absent_since: [None; 2],
        }],
        classifications: BTreeMap::from([
            (1, classified("mod_one")),
            (2, classified("mod_two")),
            (3, classified("")),
        ]),
        warnings: Vec::new(),
        unnamed_before: 0,
        unnamed_per_hour: 0,
        unnamed_warning_index: 0,
    }
}

fn kind(key: &str, ordinal: Option<usize>, domain: Option<Domain>, name: Option<&str>) -> read::Class {
    read::Class {
        key: key.into(),
        ordinal,
        domain,
        name: name.map(str::to_owned),
        reason: None,
    }
}

/// The types content gives [`page`]'s variants, by handle.
fn kinds() -> BTreeMap<u64, read::Class> {
    BTreeMap::from([
        (1, kind("mod_one", Some(0), Some(Domain::Land), Some(""))),
        (2, kind("mod_two", Some(1), Some(Domain::Air), None)),
        (3, kind(":unresolved", None, None, None)),
    ])
}

/// `kinds` as an index's classes.
fn classes(index: &read::Index, kinds: &BTreeMap<u64, read::Class>) -> read::Classes {
    read::Classes {
        of: index
            .variants
            .iter()
            .enumerate()
            .map(|(variant, handle)| {
                index.finals[variant]
                    .as_ref()
                    .and_then(|_| kinds.get(handle).cloned())
            })
            .collect(),
        raw_names: vec![None; index.raws.len()],
        sorted: read::Classes::sorted(index),
    }
}

/// `pages` completed as endpoints of branch 1 and flushed in one range.
fn published(pages: &[EndpointPage]) -> (Fixture, read::Index) {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    for page in pages {
        state.publish_page(page.clone(), None).unwrap();
    }
    state.flush(&mut writer, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let index = read::read_index(&fixture.store, &fixture.key, 1)
        .unwrap()
        .unwrap();
    (fixture, index)
}

/// The selection at `hour` of `pages` published, typed by [`kinds`].
fn choose(
    pages: &[EndpointPage],
    hour: i32,
    params: &ProducedParams,
    (offset, limit, ranked): (usize, usize, bool),
    cancelled: &AtomicBool,
) -> io::Result<select::SelectionPage> {
    let (fixture, index) = published(pages);
    select::select_with(
        &index,
        &classes(&index, &kinds()),
        &fixture.store,
        &fixture.key,
        hour,
        params,
        offset,
        limit,
        ranked,
        cancelled,
    )
    .map(|page| page.expect("an endpoint holds the hour"))
}

#[test]
fn arbitrary_subsets_keep_signed_wide_sums_ties_request_order_and_refits() {
    // regression: bdcf8aba90 Prepare resumable time-major production subset endpoints
    let flag = AtomicBool::new(false);
    let mut page = page(0, Vec::new());
    page.countries = (0..3u32)
        .map(|index| {
            let mut equipment = source(
                0,
                vec![
                    amount(1, 9_007_199_254_740_993, true, Purpose::New),
                    amount(2, -(i128::from(index) + 1), true, Purpose::New),
                    amount(1, 500, true, Purpose::Refit),
                ],
            );
            equipment.slot = index;
            equipment.country_id = index;
            let mut rocket = source(0, Vec::new());
            rocket.slot = index;
            rocket.country_id = index;
            CountryEndpoint {
                rocket_joined: true,
                absent_since: [None; 2],
                tag: format!("M{index:04}"),
                equipment: Some(equipment),
                rocket: Some(rocket),
            }
        })
        .collect();
    let pages = [page];
    let mut params = params();
    params.tags.clear();
    params.top = Some(3);
    params.included = vec!["mod_one".into(), "mod_one".into()];
    let result = choose(&pages, 0, &params, (1, 20, false), &flag).unwrap();
    assert_eq!(result.total_countries, 3);
    assert_eq!(
        result
            .countries
            .iter()
            .map(|row| row.tag.as_str())
            .collect::<Vec<_>>(),
        ["M0001", "M0002"]
    );
    assert_eq!(
        result.countries[0].total.as_deref(),
        Some("9007199254740993")
    );
    params.included.push("mod_two".into());
    params.refits = true;
    let result = choose(&pages, 0, &params, (0, 1, false), &flag).unwrap();
    assert_eq!(result.countries[0].tag, "M0000");
    assert_eq!(
        result.countries[0].total.as_deref(),
        Some("9007199254741492")
    );
    params.top = None;
    params.tags = vec!["M0002".into(), "M0000".into(), "M0002".into()];
    let result = choose(&pages, 0, &params, (0, 10, false), &flag).unwrap();
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
    let flag = AtomicBool::new(false);
    let mut page = page(
        0,
        vec![amount(1, 10, true, Purpose::New), amount(3, -3, true, Purpose::New)],
    );
    let mut params = params();
    let result = choose(&[page.clone()], 0, &params, (0, 10, false), &flag).unwrap();
    assert_eq!(result.countries[0].total.as_deref(), Some("7"));
    assert_eq!(result.countries[0].values[0].amount, None);
    params.included = vec!["mod_one".into()];
    let result = choose(&[page.clone()], 0, &params, (0, 10, false), &flag).unwrap();
    assert!(!result.known);
    assert_eq!(result.countries[0].total, None);
    params.included = vec![":unresolved".into()];
    page.countries[0].rocket = None;
    let result = choose(&[page], 0, &params, (0, 10, false), &flag).unwrap();
    assert_eq!(result.countries[0].total, None);
    assert_eq!(result.countries[0].equipment_total.as_deref(), Some("-3"));
    assert_eq!(
        result.countries[0].values[0].equipment_amount.as_deref(),
        Some("-3")
    );
}

#[test]
fn periods_use_exact_read_predecessors_and_never_bridge_holder_or_coverage_breaks() {
    let flag = AtomicBool::new(false);
    let before = page(0, vec![amount(1, -4, true, Purpose::New)]);
    let mut current = page(24, vec![amount(1, 7, true, Purpose::New)]);
    let mut params = params();
    params.measure = ProducedMeasure::Day;
    // The reader finds the day before at the current read less 24 hours.
    assert_eq!(
        predecessor(current.equipment_read.unwrap(), params.measure),
        Some(0)
    );
    let result = choose(
        &[before.clone(), current.clone()],
        30,
        &params,
        (0, 10, false),
        &flag,
    )
    .unwrap();
    assert_eq!(result.countries[0].total.as_deref(), Some("11"));
    current.countries[0].equipment.as_mut().unwrap().segment += 1;
    assert!(
        !choose(
            &[before.clone(), current.clone()],
            30,
            &params,
            (0, 10, false),
            &flag
        )
        .unwrap()
        .known
    );
    current.countries[0].equipment.as_mut().unwrap().segment -= 1;
    current.countries[0].equipment.as_mut().unwrap().country_id += 1;
    assert_eq!(
        choose(&[before, current], 30, &params, (0, 10, false), &flag)
            .unwrap()
            .countries[0]
            .total,
        None
    );
}

#[test]
fn manual_rank_pages_sort_wide_scores_before_paging_and_keep_the_default_request_order() {
    let flag = AtomicBool::new(false);
    let mut page = page(0, vec![amount(1, 9_007_199_254_740_992, true, Purpose::New)]);
    let mut second = page.countries[0].clone();
    second.tag = "B".into();
    second.equipment.as_mut().unwrap().slot = 2;
    second.equipment.as_mut().unwrap().country_id = 3;
    second.rocket.as_mut().unwrap().slot = 2;
    second.rocket.as_mut().unwrap().country_id = 3;
    second.equipment.as_mut().unwrap().amounts[0].units = 9_007_199_254_740_993;
    page.countries.push(second);
    let pages = [page];
    let mut params = params();
    params.tags.push("B".into());
    assert_eq!(
        choose(&pages, 0, &params, (0, 1, false), &flag)
            .unwrap()
            .countries[0]
            .tag,
        "A"
    );
    assert_eq!(
        choose(&pages, 0, &params, (0, 1, true), &flag)
            .unwrap()
            .countries[0]
            .tag,
        "B"
    );
    // No read a day before the calendar's first: no period is compatible there.
    assert_eq!(predecessor(i32::MIN, ProducedMeasure::Day), None);
}

#[test]
fn cancelled_queries_refuse_before_returning_partial_selection() {
    assert_eq!(
        choose(
            &[page(0, Vec::new())],
            0,
            &params(),
            (0, 1, false),
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
        vec![amount(1, 10, true, Purpose::New), amount(2, 7, true, Purpose::New)],
    );
    let mut second = current.countries[0].clone();
    second.tag = "B".into();
    second.equipment.as_mut().unwrap().amounts =
        vec![amount(1, 2, true, Purpose::New), amount(2, 20, true, Purpose::New)];
    current.countries.push(second);
    let mut input = params();
    input.tags.push("B".into());
    let selected = choose(&[current], 0, &input, (0, 0, false), &AtomicBool::new(false)).unwrap();
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
    let mut early = page(0, vec![amount(1, 4, true, Purpose::New)]);
    early.last_hour = 7;
    // One type, so it is the first key.
    early.classifications.retain(|variant, _| *variant == 1);
    let mut open = page(8, vec![amount(1, 5, true, Purpose::New)]);
    open.last_hour = 8;
    open.classifications.retain(|variant, _| *variant == 1);
    let eq = open.countries[0].equipment.clone().unwrap();
    assert!(
        state
            .joins
            .bind_rocket("A", &eq, open.countries[0].rocket.as_ref(), 8, None)
    );
    let mut writer = store.replace(key.clone()).unwrap();
    state.publish_page(early, None).unwrap();
    state.publish_page(open, None).unwrap();
    state.flush(&mut writer, &flag).unwrap();
    writer.commit().unwrap();
    // Without content, a type is its recorded token under the unresolved mark.
    let at = |hour: i32| {
        let index = read::read_index(&store, &key, 1).unwrap().unwrap();
        let classes = read::Classes::new(&index, Named::default());
        select::select_with(
            &index,
            &classes,
            &store,
            &key,
            hour,
            &params(),
            0,
            1,
            false,
            &flag,
        )
        .unwrap()
        .unwrap()
    };
    let before = at(8);
    assert_eq!(before.countries[0].total.as_deref(), Some("5"));
    assert_eq!(
        before.keys[0].key,
        format!("{}mod_one", equipment::UNRESOLVED_MARK)
    );
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
    // The Rocket read supplying the join ends: the join is withdrawn.
    assert!(!state.joins.bind_rocket("A", &eq, None, 9, None));
    state.classifications.get_mut(&1).unwrap().token = "mod_reclassified".into();
    let mut writer = store.extend(&key).unwrap();
    state.flush(&mut writer, &flag).unwrap();
    writer.commit().unwrap();
    let after = at(8);
    assert_eq!(
        after.countries[0].total, None,
        "the next commit withdraws the open supplying interval's join"
    );
    assert_eq!(
        after.countries[0].equipment_total.as_deref(),
        Some("5"),
        "the equipment source stays known"
    );
    assert_eq!(
        after.keys[0].key,
        format!("{}mod_reclassified", equipment::UNRESOLVED_MARK),
        "later classification revises the same earlier hour"
    );
    let unaffected = at(0);
    assert_eq!(unaffected.countries[0].total.as_deref(), Some("4"));
    drop(recording);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pages_before_the_first_supply_count_from_the_observation_history_lists() {
    let flag = AtomicBool::new(false);
    // The first publications carry no output; the first supply, read later,
    // dates its counting from the branch start.
    let mut early = page(0, Vec::new());
    early.countries.clear();
    early.equipment_origin = None;
    let later = page(24, vec![amount(1, 7, true, Purpose::New)]);
    let at = |fixture: &Fixture, hour: i32, selection: &ProducedParams| {
        let index = read::read_index(&fixture.store, &fixture.key, 1)
            .unwrap()
            .unwrap();
        select::select_with(
            &index,
            &classes(&index, &kinds()),
            &fixture.store,
            &fixture.key,
            hour,
            selection,
            0,
            5,
            false,
            &flag,
        )
        .unwrap()
        .unwrap()
    };
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    state.publish_page(early.clone(), None).unwrap();
    state.publish_page(later.clone(), None).unwrap();
    state.result.observations = vec![0];
    write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    let mut top = params();
    top.tags.clear();
    top.top = Some(5);
    for (hour, selection) in [(12, params()), (12, top.clone()), (30, params()), (30, top)] {
        let page = at(&fixture, hour, &selection);
        assert_eq!(page.observation, Some(0), "hour {hour}, top {:?}", selection.top);
        assert!(!page.recorded, "no production read yet, hour {hour}");
    }
    // A restart is dated from its own counting start onwards.
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    state.publish_page(early, None).unwrap();
    state.publish_page(later, None).unwrap();
    state.result.observations = vec![0];
    state.observation_starts = state.observation_starts();
    state.result.observations = vec![0, 20];
    state.result.reads = vec![24];
    write_state(&mut writer, &mut state).unwrap();
    writer.commit().unwrap();
    let mut types = params();
    types.tags.clear();
    for (hour, since, selection) in [(12, 0, params()), (30, 20, params()), (12, 0, types)] {
        let page = at(&fixture, hour, &selection);
        assert_eq!(page.observation, Some(since), "hour {hour}");
        // Even before its first read, a page says the branch has production.
        assert!(page.recorded, "hour {hour}, tags {:?}", selection.tags);
    }
}

#[test]
fn persisted_pages_and_resume_states_isolate_overlapping_branch_hours() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let first = page(0, vec![amount(1, 7, true, Purpose::New)]);
    let second = page(0, vec![amount(1, 9, true, Purpose::New)]);
    let mut resumed = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    let mut other_branch = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 2, 0);
    resumed.publish_page(first.clone(), None).unwrap();
    other_branch.publish_page(second, None).unwrap();
    write_state(&mut writer, &mut resumed).unwrap();
    // A later flush reclassifies the variant: the publication's last wins.
    resumed.classifications.get_mut(&1).unwrap().token = "mod_final".into();
    write_state(&mut writer, &mut resumed).unwrap();
    write_state(&mut writer, &mut other_branch).unwrap();
    writer.commit().unwrap();
    let cancelled = AtomicBool::new(false);
    let at = |branch: u64, hour: i32, params: &ProducedParams| {
        let index = read::read_index(&fixture.store, &fixture.key, branch)
            .unwrap()
            .unwrap();
        let classes = read::Classes::new(&index, Named::default());
        select::select_with(
            &index,
            &classes,
            &fixture.store,
            &fixture.key,
            hour,
            params,
            0,
            10,
            false,
            &cancelled,
        )
        .unwrap()
        .unwrap()
    };
    let reclassified = format!("{}mod_final", equipment::UNRESOLVED_MARK);
    let mut params = params();
    params.included = vec![reclassified.clone()];
    assert_eq!(
        at(1, 12, &params).countries[0].total.as_deref(),
        Some("7")
    );
    let mut types_only = params.clone();
    types_only.tags.clear();
    let types = at(1, 999, &types_only);
    assert!(types.countries.is_empty());
    assert!(types.keys.is_empty());
    assert!(types.types.iter().any(|kind| kind.key == reclassified));
    params.branch_id = "2".into();
    params.included.clear();
    assert_eq!(
        at(2, 12, &params).countries[0].total.as_deref(),
        Some("9")
    );
    assert_eq!(
        latest_state(&fixture.store, &fixture.key, 1)
            .unwrap()
            .unwrap()
            .branch,
        1
    );
    assert_eq!(
        latest_state(&fixture.store, &fixture.key, 2)
            .unwrap()
            .unwrap()
            .branch,
        2
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, &section(STATE, 1))
            .unwrap(),
        Some(1)
    );
    assert!(
        read::read_index(&fixture.store, &fixture.key, 3)
            .unwrap()
            .is_none()
    );
}

/// What `page` publishes for each of its holders, by the index's ids.
fn expected_holders(index: &read::Index, page: &EndpointPage) -> BTreeMap<u32, facts::Holder> {
    let source = |source: &SourceEndpoint, read: Option<i32>| facts::Source {
        head: facts::Head {
            spending_since: source.spending_since,
            slot: source.slot,
            country_id: source.country_id,
            observation: source.observation,
            segment: source.segment,
            read_hour: (Some(source.read_hour) != read).then_some(source.read_hour),
            units_known: source.units_known,
            spending_known: source.spending_known,
            present: source.present,
            campaign_reason: source.campaign_spending_reason,
        },
        leaves: {
            let mut leaves: Vec<facts::Leaf> = source
                .amounts
                .iter()
                .map(|amount| facts::Leaf {
                    variant: index.variant_id(amount.variant).unwrap(),
                    purpose: amount.purpose,
                    units: i64::try_from(amount.units).unwrap(),
                    units_exact: amount.units_exact,
                    spent: i64::try_from(amount.spent).unwrap(),
                    reason: amount.spending_reason,
                    unproven: i64::try_from(amount.unproven).unwrap(),
                })
                .collect();
            leaves.sort_by_key(facts::Leaf::key);
            leaves
        },
    };
    page.countries
        .iter()
        .map(|row| {
            (
                index.tags[&row.tag],
                facts::Holder {
                    equipment: row
                        .equipment
                        .as_ref()
                        .map(|equipment| source(equipment, page.equipment_read)),
                    rocket: row
                        .rocket
                        .as_ref()
                        .map(|rocket| source(rocket, page.rocket_read)),
                    rocket_joined: row.rocket_joined,
                    absent_since: row.absent_since,
                },
            )
        })
        .collect()
}

#[test]
fn published_endpoints_decode_identically_after_two_commits_resume_append_and_retirement() {
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    // The widest amount kept at rest exactly.
    let wide = i128::from(i64::MAX) - 17;
    let mut expected = Vec::new();
    for block in 0..5 {
        let read = block * 24;
        let mut current = page(
            read,
            vec![amount(1, wide, true, Purpose::New), amount(2, -31, false, Purpose::Refit)],
        );
        current.countries[0].rocket.as_mut().unwrap().amounts =
            vec![amount(0, -wide + i128::from(block), true, Purpose::New)];
        if block >= 1 {
            // Dated by the read at hour 24, which is the names hour only there.
            current.classifications.get_mut(&1).unwrap().names = vec![VariantName {
                first_hour: 24,
                raw: None,
            }];
            current.countries[0].equipment.as_mut().unwrap().amounts[1].spending_reason =
                Some(ProductionSpendingUnknown::LostOutput);
        }
        if block == 2 {
            current.countries[0].equipment = None;
            current.countries[0].absent_since[0] = Some(read);
            current.countries[0].rocket_joined = false;
        }
        if block >= 3 {
            let equipment = current.countries[0].equipment.as_mut().unwrap();
            equipment.slot = 7;
            equipment.country_id = 19;
            equipment.segment = 2;
            equipment.present = None;
            current.countries[0].rocket = None;
            current.countries[0].absent_since[1] = Some(72);
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
        state.publish_page(current.clone(), Some(read)).unwrap();
        expected.push(current);
        if [1, 3, 4].contains(&block) {
            write_state(&mut writer, &mut state).unwrap();
            writer.commit().unwrap();
            let index = read::read_index(&fixture.store, &fixture.key, 1)
                .unwrap()
                .unwrap();
            let classes = read::Classes::new(&index, Named::default());
            assert_eq!(index.endpoints.len(), expected.len());
            for (ordinal, original) in expected.iter().enumerate() {
                let ordinal = ordinal as u64;
                for hour in [original.first_hour, original.last_hour] {
                    assert_eq!(index.endpoint_at(hour), Some(ordinal));
                }
                let wanted = expected_holders(&index, original);
                let decoded: BTreeMap<u32, facts::Holder> = index
                    .state(&fixture.store, &fixture.key, ordinal, &flag)
                    .unwrap()
                    .holders
                    .iter()
                    .enumerate()
                    .filter_map(|(id, holder)| holder.clone().map(|holder| (id as u32, holder)))
                    .collect();
                assert_eq!(decoded, wanted, "ranking state at {ordinal} after block {block}");
                for (&holder, state) in &wanted {
                    assert_eq!(
                        index
                            .holder_at(&fixture.store, &fixture.key, holder, ordinal)
                            .unwrap()
                            .as_ref(),
                        Some(state),
                        "holder leaves at {ordinal} after block {block}"
                    );
                }
                let variant = index.variant_id(1).unwrap();
                let names = index.names_at(&classes, variant, ordinal).unwrap().unwrap();
                if ordinal == 0 {
                    assert!(names.is_empty());
                } else {
                    assert_eq!(
                        names,
                        [StockVariantName {
                            first_hour: 24,
                            name: None,
                            known: false,
                            design: None,
                        }],
                        "names at {ordinal} after block {block}"
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
}

#[test]
fn amounts_outside_the_exact_range_at_rest_are_refused_never_truncated() {
    let mut state = ProductionAccumulator::new(replay_data::FORMAT_VERSION, 1, 0);
    let wide = page(0, vec![amount(1, 1i128 << 100, true, Purpose::New)]);
    assert!(state.publish_page(wide, None).is_err());
}

#[test]
fn a_changed_leaf_publishes_the_same_ranking_bytes_however_many_leaves_retired_before() {
    let flag = AtomicBool::new(false);
    let mut current = page(0, Vec::new());
    current.classifications.clear();
    let mut pages = Vec::new();
    // Each endpoint introduces one variant and keeps every earlier total unchanged.
    for block in 1..=41u64 {
        current.first_hour = block as i32 * 720;
        current.last_hour = current.first_hour + 719;
        current.equipment_read = Some(current.first_hour);
        let equipment = current.countries[0].equipment.as_mut().unwrap();
        equipment.read_hour = current.first_hour;
        equipment.amounts.push(amount(block, 17, true, Purpose::New));
        current
            .classifications
            .insert(block, classified(&format!("mod variant {block:04}")));
        pages.push(current.clone());
    }
    let (fixture, index) = published(&pages);
    assert!(index.endpoint(16).keyframe && index.endpoint(32).keyframe);
    let sizes = [3u64, 39].map(|ordinal| {
        let endpoint = index.endpoint(ordinal);
        assert!(!endpoint.keyframe);
        endpoint.ranking_len
    });
    assert!(
        sizes[1].abs_diff(sizes[0]) <= 8,
        "a changed endpoint's block grew with retired history: {sizes:?}"
    );
    let state = index.state(&fixture.store, &fixture.key, 39, &flag).unwrap();
    let holder = state.holder(index.tags["A"]).unwrap();
    assert_eq!(holder.equipment.as_ref().unwrap().leaves.len(), 40);
}

#[test]
fn null_equipment_design_keeps_units_and_unresolved_attribution() {
    let flag = AtomicBool::new(false);
    let mut current = page(0, vec![amount(0, 7, true, Purpose::New)]);
    current.classifications.insert(0, classified(""));
    let (fixture, index) = published(&[current]);
    let mut types = kinds();
    types.insert(0, kind(":unresolved", None, None, None));
    let classes = classes(&index, &types);
    let at = |request: &ProducedParams| {
        select::select_with(
            &index,
            &classes,
            &fixture.store,
            &fixture.key,
            0,
            request,
            0,
            1,
            false,
            &flag,
        )
        .unwrap()
        .unwrap()
    };
    let mut request = params();
    assert_eq!(at(&request).countries[0].total.as_deref(), Some("7"));
    request.included = vec!["mod_one".into()];
    assert_eq!(at(&request).countries[0].total, None);
}

#[test]
fn branch_aliases_compare_numerically_and_echo_the_request() {
    let current = page(0, vec![amount(1, 7, true, Purpose::New)]);
    for alias in ["01", "+1"] {
        let mut request = params();
        request.branch_id = alias.into();
        let selected = choose(
            &[current.clone()],
            0,
            &request,
            (0, 1, false),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(selected.branch_id, alias);
        assert_eq!(selected.countries[0].total.as_deref(), Some("7"));
    }
}

#[test]
fn global_value_extent_includes_off_page_signed_rows_exactly() {
    let mut current = page(0, vec![amount(1, 9007199254740993, true, Purpose::New)]);
    let mut negative = current.countries[0].clone();
    negative.tag = "B".into();
    negative.equipment.as_mut().unwrap().amounts = vec![amount(1, -7, true, Purpose::New)];
    current.countries.push(negative);
    let mut request = params();
    request.tags.push("B".into());
    let selected = choose(&[current], 0, &request, (1, 1, false), &AtomicBool::new(false)).unwrap();
    assert_eq!(selected.offset, "1");
    assert_eq!(selected.value_min, "-7");
    assert_eq!(selected.value_max, "9007199254740993");
    assert_eq!(selected.countries.len(), 1);
    assert_eq!(selected.countries[0].tag, "B");
}

#[test]
fn production_spending_projects_equal_inventories_once_and_rebuilds_after_restore() {
    use crate::prepared::series::tests::assert_payloads_equal;
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
            purpose: Recorded::Value(OutputPurpose::New.raw()),
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
            state.ingest(format, &frame, &flag).unwrap();
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
    use crate::prepared::{frame_changes::FrameChanges, series::tests::assert_payloads_equal};
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
                    purpose: Recorded::Value(OutputPurpose::New.raw()),
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
                            let completed = state.completed.len();
                            state
                                .ingest_changed(format, &frame, &flag, &changes)
                                .unwrap();
                            assert_eq!(
                                state.completed.len(),
                                completed,
                                "a carried hour completes no endpoint"
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
                    .ingest_changed(format, &frame, &flag, &changes)
                    .unwrap();
                // A whole projection visits both inventory keys of every design; the
                // changed owner's projection only those of its changed design.
                let visits = if index == 1 || day == 0 {
                    4 * VARIANTS
                } else {
                    2 * u32::from(day == VARIANTS / 2)
                };
                assert_eq!(
                    state.spent_visits - before,
                    visits as usize,
                    "quiet={quiet}, day={day}: only changed designs project"
                );
            }
            assert_eq!(
                serde_json::to_vec(&states[0]).unwrap(),
                serde_json::to_vec(&states[1]).unwrap()
            );
        }
        assert_eq!(states[0].spent_visits, (4 * VARIANTS + 2) as usize);
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
                    purpose: Recorded::Value(OutputPurpose::New.raw()),
                    ..Default::default()
                })
                .collect(),
        ));
    let format = observer_format(&frame);
    let mut state = ProductionAccumulator::new(format.version(), 1, first);
    let mut tracker = FrameChangeTracker::new();
    frame.metrics.table_ns[TableId::ProductionLineOutput.index()] = 1;
    state
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
        .unwrap();
    assert_eq!(state.variants.inventory_visits, 2 * count as usize);
    for offset in 1..=4 {
        frame.hour = first + offset;
        frame.metrics.table_ns.fill(0);
        // A real presentation read with unchanged numeric inventories.
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
        let before = state.variants.inventory_visits;
        state
            .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
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
        .ingest_changed(format, &frame, &flag, &tracker.observe(&frame, format))
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
fn source_absence_continues_through_saved_state_and_resets_on_gap_or_reappearance() {
    let mut saved = None;
    let mut head = SourceHead::of(&source(100, Vec::new()));
    SpendingSource::whole(&mut saved, &mut head, &[], false);
    SpendingSource::absent(&mut saved, 120, true);
    assert_eq!(saved.as_ref().unwrap().since, 120);
    let bytes = serde_json::to_vec(&saved).unwrap();
    let mut saved: Option<SpendingSource> = serde_json::from_slice(&bytes).unwrap();
    SpendingSource::absent(&mut saved, 130, true);
    assert_eq!(saved.as_ref().unwrap().since, 120);
    SpendingSource::absent(&mut saved, 140, false);
    assert_eq!(saved.as_ref().unwrap().since, 140);
    head.spending_since = 150;
    SpendingSource::whole(&mut saved, &mut head, &[], true);
    assert_eq!(head.spending_since, 150);
    SpendingSource::absent(&mut saved, 160, true);
    assert_eq!(saved.as_ref().unwrap().since, 160);
}
