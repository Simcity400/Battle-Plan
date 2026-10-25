use super::*;
use crate::production::design_changes::tests::{append, edit, fixture};
use crate::production::production_rest_test_kit as kit;

fn burst(count: usize) -> Batch {
    let mut batch = Batch::empty();
    batch.prepare(count, count * 128, count * ROOT_BYTES * 4);
    let mut bytes = fixture();
    for index in 0..count {
        edit(&mut bytes, b'A' + (index % 8) as u8, index as i64 % 8 - 4);
        append(&mut batch, &bytes, index as u64 + 1, false);
    }
    batch
}
#[test]
fn a_large_repeated_burst_keeps_every_ordered_row_and_materializes_the_original_bytes() {
    let mut batch = burst(4_096);
    let originals: Vec<_> = batch.events[..8].to_vec();
    for index in 0..batch.events.len() {
        let source = originals[index / 4 % 8];
        batch.events[index].start = source.start;
        batch.events[index].end = source.end;
    }
    let mut rows = Vec::new();
    let mut shared = Shared::default();
    decode_batch_shared(&batch, 0, 7, &mut rows, &mut shared).unwrap();
    assert!(rows.is_empty());
    assert_eq!(shared.entries.len(), batch.events.len());
    for index in 0..shared.len() {
        assert!(Arc::ptr_eq(
            &shared.entries[index].value,
            &shared.entries[(index / 4 % 8) * 4].value
        ));
    }
    assert!(!Arc::ptr_eq(
        &shared.entries[0].value,
        &shared.entries[4].value
    ));
    let expected = batch
        .events
        .iter()
        .zip(shared.iter(&rows))
        .map(|(event, change)| {
            let mut row = change.unwrap().try_clone().unwrap();
            let memory =
                crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
            row.snapshot = Some(snapshot(&memory, 0, 7, kit::VARIANT, false).unwrap());
            row
        })
        .collect::<Vec<_>>();
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows {
        len: expected.len(),
    };
    captured.snapshots = shared;
    let mut owned = Captured::vacant();
    owned.outcome = captured.outcome;
    owned.rows = expected.clone();
    for keyframe in [false, true] {
        for existing in [false, true] {
            assert_eq!(
                converted_bytes(&captured, keyframe, existing),
                converted_bytes(&owned, keyframe, existing)
            );
        }
    }
    shared = captured.snapshots;
    shared.materialize(&mut rows).unwrap();
    assert_eq!(rows, expected);
    for row in &rows {
        rows::RowCodec::check_key_fields(row).unwrap();
    }
    let mut actual_bytes = Vec::new();
    let mut expected_bytes = Vec::new();
    for (actual, expected) in rows.iter().zip(&expected) {
        rows::encode_row(actual, 0, &mut actual_bytes).unwrap();
        rows::encode_row(expected, 0, &mut expected_bytes).unwrap();
    }
    assert_eq!(actual_bytes, expected_bytes);
}
#[test]
fn colliding_fingerprints_still_require_identical_scoped_bytes_and_valid_native_inputs() {
    let batch = burst(9);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut decode = |index: usize| {
        let event = &batch.events[index];
        cache
            .decode_key(&batch, event, 0, 7, &mut shared, 0)
            .unwrap()
    };
    let first = decode(0);
    let changed = decode(1);
    let repeated = decode(8);
    assert!(Arc::ptr_eq(&first, &repeated));
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(cache.entries.len(), 2);
    cache.heads.clear();
    let recent = cache
        .decode(&batch, &batch.events[1], 0, 7, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&changed, &recent));
    assert!(cache.heads.is_empty());
    let event = Event {
        pointer: kit::VARIANT + 1,
        ..batch.events[8]
    };
    assert!(
        cache
            .decode_key(&batch, &event, 0, 7, &mut shared, 0)
            .is_err()
    );
}
#[test]
fn repeated_and_overlapping_witnesses_must_be_single_valued_before_reusing_fresh_cursors() {
    let mut log = RawLog::<1>::new(4, 64);
    let first = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    let same = engine_native::memory::SliceMemory {
        base: 0x1001,
        bytes: &[2, 3],
    };
    let changed = engine_native::memory::SliceMemory {
        base: 0x1001,
        bytes: &[2, 9],
    };
    log.take_bulk(&first, 0x1000, 4, 1).unwrap();
    log.take_bulk(&same, 0x1001, 2, 1).unwrap();
    let mut cache = Cache::default();
    assert!(cache.consistent(&log, 0..2).unwrap());
    log.take_bulk(&changed, 0x1001, 2, 1).unwrap();
    assert!(!cache.consistent(&log, 0..3).unwrap());
    assert!(cache.consistent(&log, 0..1).unwrap());
    log.spans[0].ok = false;
    assert!(!cache.consistent(&log, 0..1).unwrap());
}
#[test]
fn a_cached_snapshot_does_not_bypass_occurrence_order_refusal() {
    let mut batch = burst(32);
    batch.events[24].sequence = batch.events[23].sequence;
    let mut output = Vec::new();
    let mut shared = Shared::default();
    assert_eq!(
        decode_batch_shared(&batch, 0, 7, &mut output, &mut shared),
        Err("variant occurrences are not in native observation order")
    );
    assert!(output.is_empty());
    assert_eq!(shared.len(), 24);
}

#[test]
fn shared_history_preserves_flags_removal_and_republication_in_native_order() {
    let mut batch = burst(6);
    batch.events[1].flags = Some([1, 2, 3]);
    batch.events[2].removed = true;
    batch.events[3].start = batch.events[0].start;
    batch.events[3].end = batch.events[0].end;
    let mut output = Vec::new();
    let mut shared = Shared::default();
    decode_batch_shared(&batch, 0, 7, &mut output, &mut shared).unwrap();
    assert!(Arc::ptr_eq(
        &shared.entries[0].value,
        &shared.entries[1].value
    ));
    let mut history = output.clone();
    shared.materialize(&mut history).unwrap();
    assert_eq!(history.len(), batch.events.len());
    assert_eq!(history[1].flags, Some([1, 2, 3]));
    assert!(history[1].snapshot.is_none());
    assert!(history[2].removed);
    assert!(history[2].snapshot.is_none());
    assert_eq!(history[0].snapshot, history[3].snapshot);
    assert!(
        history
            .windows(2)
            .all(|rows| rows[0].sequence < rows[1].sequence)
    );
    output[0].variant = 0;
    assert!(shared.materialize(&mut output).is_err());
}

fn converted_bytes(captured: &Captured, keyframe: bool, existing: bool) -> Vec<u8> {
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};
    let frame = |hour| FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
        metrics: Metrics::default(),
    };
    let mut first = frame(6);
    for (table, rows) in [
        (
            TableId::VariantDefinitions,
            rows::TableRows::VariantDefinitions(Vec::new()),
        ),
        (
            TableId::VariantModules,
            rows::TableRows::VariantModules(Vec::new()),
        ),
        (
            TableId::VariantStats,
            rows::TableRows::VariantStats(Vec::new()),
        ),
        (
            TableId::VariantUpgrades,
            rows::TableRows::VariantUpgrades(Vec::new()),
        ),
    ] {
        first.tables[table.index()] = TableView::Complete(rows).into();
    }
    if existing {
        let row = captured
            .history()
            .iter()
            .find_map(|change| {
                let change = change.unwrap();
                change.view().snapshot.map(|_| change.try_clone().unwrap())
            })
            .unwrap();
        let snapshot = row.snapshot.unwrap();
        let state = snapshot
            .definition
            .design
            .require("fixture design")
            .unwrap();
        first.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(rows::TableRows::VariantDefinitions(vec![
                snapshot.try_definition().unwrap(),
            ]))
            .into();
        first.tables[TableId::VariantModules.index()] =
            TableView::Complete(rows::TableRows::VariantModules(
                snapshot
                    .modules
                    .iter()
                    .map(|row| rows::VariantModuleRow {
                        variant: snapshot.definition.id,
                        source_ordinal: row.ordinal,
                        slot: row.slot,
                        definition_token: row.definition_token,
                        definition_active: row.active,
                    })
                    .collect(),
            ))
            .into();
        first.tables[TableId::VariantStats.index()] =
            TableView::Complete(rows::TableRows::VariantStats(vec![rows::VariantStatsRow {
                variant: snapshot.definition.id,
                organisation: state.design_team,
                organisation_trait_count: state.design_team_traits,
                organisation_bonuses: state
                    .design_team_bonus
                    .iter()
                    .map(|entry| rows::IndustrialOrganisationBonus {
                        key: entry.index,
                        value_raw: entry.value,
                    })
                    .collect(),
                values: snapshot.stats.as_slice().try_into().unwrap(),
            }]))
            .into();
        first.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(rows::TableRows::VariantUpgrades(
                snapshot
                    .upgrades
                    .iter()
                    .map(|row| rows::VariantUpgradeRow {
                        variant: snapshot.definition.id,
                        source_ordinal: row.ordinal,
                        definition_token: row.definition_token,
                        level: row.level,
                    })
                    .collect(),
            ))
            .into();
    }
    let mut encoder = FrameEncoder::new();
    encoder.encode_input(first, false).unwrap();
    let scratch = crate::projection::ProjectionScratch::default();
    let projection = Projection::new(&encoder, 1, 7, keyframe, &scratch);
    let mut second = frame(7);
    for (table, input) in [
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantStats,
        TableId::VariantUpgrades,
    ]
    .into_iter()
    .zip(project_history(
        captured,
        captured.history(),
        Some(&projection),
    )) {
        second.tables[table.index()] = input;
    }
    second.tables[TableId::VariantChanges.index()] = if captured.snapshots.len() == 0 {
        let mut previous = HashMap::new();
        let mut output = Vec::new();
        for (index, row) in captured.rows.iter().enumerate() {
            let preceding = previous
                .get(&row.variant)
                .map_or(&[][..], |&last| &captured.rows[last..last + 1]);
            if !owned_unchanged(row, preceding, Some(&projection)) {
                output.push(row.clone());
            }
            previous.insert(row.variant, index);
        }
        TableView::Complete(rows::TableRows::VariantChanges(output)).into()
    } else {
        captured.table_input(Some(&projection))
    };
    let mut bytes = Vec::new();
    encoder
        .encode_input(second, keyframe)
        .unwrap()
        .encode(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn suppressed_occurrences_allocate_nothing_after_the_first_emitted_row() {
    let mut batch = burst(4_096);
    for event in &mut batch.events {
        event.start = 0;
        event.end = batch.log.spans.len() / 4_096;
    }
    let mut rows = Vec::new();
    let mut shared = Shared::default();
    decode_batch_shared(&batch, 0, 7, &mut rows, &mut shared).unwrap();
    let mut changes = shared.iter(&rows);
    let mut output = Vec::new();
    let mut previous = HashMap::new();
    append_history(changes.by_ref().take(1), &mut output, &mut previous, None).unwrap();
    let capacity = output.capacity();
    let map_capacity = previous.capacity();
    collector_test_support::with_allocations_denied(|| {
        append_history(changes, &mut output, &mut previous, None)
    })
    .unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output.capacity(), capacity);
    assert_eq!(previous.capacity(), map_capacity);
}

#[test]
fn different_shared_allocations_still_compare_deeply_and_emitted_rows_validate_keys() {
    let batch = burst(2);
    let mut rows = Vec::new();
    let mut shared = Shared::default();
    decode_batch_shared(&batch, 0, 7, &mut rows, &mut shared).unwrap();
    let first = &shared.entries[0];
    let memory =
        crate::raw::Recorded::build_range(&batch.log, batch.events[0].start..batch.events[0].end)
            .unwrap();
    let equal = Occurrence {
        index: 1,
        sequence: first.sequence + 1,
        hour: first.hour,
        variant: first.variant,
        value: Arc::new(decode_snapshot(&memory, 0, 7, kit::VARIANT, false).unwrap()),
    };
    assert!(!Arc::ptr_eq(&first.value, &equal.value));
    assert!(unchanged(
        Change::Snapshot(&equal),
        Some(Change::Snapshot(first)),
        None
    ));
    assert!(!unchanged(
        Change::Snapshot(&shared.entries[1]),
        Some(Change::Snapshot(first)),
        None
    ));
    let owned = Change::Snapshot(first).try_clone().unwrap();
    assert!(unchanged(
        Change::Snapshot(&equal),
        Some(Change::Row(&owned)),
        None
    ));
    assert!(unchanged(
        Change::Row(&owned),
        Some(Change::Snapshot(&equal)),
        None
    ));
    let invalid = Occurrence {
        variant: 0,
        ..equal
    };
    assert!(Change::Snapshot(&invalid).try_clone().is_err());
}

fn owned_unchanged(
    event: &rows::VariantChangeRow,
    preceding: &[rows::VariantChangeRow],
    projection: Option<&Projection<'_>>,
) -> bool {
    if event.removed {
        return false;
    }
    let Some(snapshot) = &event.snapshot else {
        return false;
    };
    if let Some(previous) = preceding
        .iter()
        .rev()
        .find(|row| row.variant == event.variant)
    {
        return !previous.removed && previous.snapshot.as_ref() == Some(snapshot);
    }
    let Some(projection) = projection else {
        return false;
    };
    let Some(definitions) = projection.observed_baseline::<tables::VariantDefinitions>() else {
        return false;
    };
    let Ok(index) = definitions.binary_search_by_key(&event.variant, |row| row.id) else {
        return false;
    };
    if !snapshot.definition.matches(&definitions[index]) {
        return false;
    }
    let (Some(modules), Some(stats), Some(upgrades)) = (
        projection.observed_baseline::<tables::VariantModules>(),
        projection.observed_baseline::<tables::VariantStats>(),
        projection.observed_baseline::<tables::VariantUpgrades>(),
    ) else {
        return false;
    };
    let module_start = modules.partition_point(|row| row.variant < event.variant);
    let upgrade_start = upgrades.partition_point(|row| row.variant < event.variant);
    let Ok(stat) = stats.binary_search_by_key(&event.variant, |row| row.variant) else {
        return false;
    };
    stats[stat].values.as_slice() == snapshot.stats.as_slice()
        && modules[module_start..]
            .iter()
            .take_while(|row| row.variant == event.variant)
            .map(|r| {
                (
                    r.source_ordinal,
                    r.slot,
                    r.definition_token,
                    r.definition_active,
                )
            })
            .eq(snapshot
                .modules
                .iter()
                .map(|r| (r.ordinal, r.slot, r.definition_token, r.active)))
        && upgrades[upgrade_start..]
            .iter()
            .take_while(|row| row.variant == event.variant)
            .map(|r| (r.source_ordinal, r.definition_token, r.level))
            .eq(snapshot
                .upgrades
                .iter()
                .map(|r| (r.ordinal, r.definition_token, r.level)))
}

#[test]
fn mixed_occurrences_keep_scalar_sources_flags_liveness_removal_and_owned_snapshot_positions() {
    let batch = burst(2);
    let mut rows = Vec::new();
    let mut decoded = Shared::default();
    decode_batch_shared(&batch, 0, 7, &mut rows, &mut decoded).unwrap();
    let first = &decoded.entries[0].value;
    let variant = first.value.definition.id;
    let snapshot = |value: &Arc<Decoded>| rows::VariantChangeRow {
        variant,
        snapshot: Some(value.value.clone()),
        ..Default::default()
    };
    let mut history = vec![
        snapshot(first),
        snapshot(first),
        rows::VariantChangeRow {
            variant,
            scalar: Some(rows::VariantScalarChange {
                display_name: Some(b"scalar".to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        },
        snapshot(first),
        rows::VariantChangeRow {
            variant,
            sources: Some(rows::VariantSourceReferences {
                direct: vec![1],
                named: vec![2],
            }),
            ..Default::default()
        },
        snapshot(first),
        rows::VariantChangeRow {
            variant,
            flags: Some([1, 0, 1]),
            ..Default::default()
        },
        rows::VariantChangeRow {
            organisation: Some(rows::VariantOrganisationChange {
                organisation: 79,
                live: true,
            }),
            ..Default::default()
        },
        rows::VariantChangeRow {
            variant,
            removed: true,
            ..Default::default()
        },
        snapshot(first),
        snapshot(first),
        snapshot(&decoded.entries[1].value),
        snapshot(&decoded.entries[1].value),
    ];
    let mut shared = Captured::vacant();
    shared.outcome = Outcome::Rows { len: history.len() };
    for (index, row) in history.iter_mut().enumerate() {
        row.sequence = index as u64 + 1;
        row.hour = 7;
        if row.snapshot.is_some() && index != 12 {
            let value = if index == 11 {
                &decoded.entries[1].value
            } else {
                first
            };
            shared
                .snapshots
                .push(
                    index,
                    row.sequence,
                    row.hour,
                    row.variant,
                    Arc::clone(value),
                )
                .unwrap();
        } else {
            shared.rows.push(row.clone());
        }
    }
    let mut owned = Captured::vacant();
    owned.outcome = shared.outcome;
    owned.rows = history;
    for keyframe in [false, true] {
        for existing in [false, true] {
            assert_eq!(
                converted_bytes(&shared, keyframe, existing),
                converted_bytes(&owned, keyframe, existing)
            );
        }
    }
    let TableInput::Sampled(TableView::Complete(rows::TableRows::VariantChanges(rows))) =
        shared.table_input(None)
    else {
        panic!("expected complete changes");
    };
    assert_eq!(
        rows.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        [1, 3, 4, 5, 6, 7, 8, 9, 10, 12]
    );
}

#[test]
fn unread_copied_bytes_do_not_redecode_repeated_designs_or_change_their_rows() {
    let count = 4_096;
    let mut batch = Batch::empty();
    batch.prepare(count, count * 128, count * ROOT_BYTES * 4);
    let mut bytes = fixture();
    for index in 0..count {
        edit(&mut bytes, b'A' + (index % 8) as u8, index as i64 % 8);
        let start = kit::VARIANT - kit::BASE + native::UPGRADES + 24;
        bytes[start..start + 8].copy_from_slice(&(index as u64).to_le_bytes());
        append(&mut batch, &bytes, index as u64 + 1, false);
    }
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut expected = Vec::new();
    for (index, event) in batch.events.iter().enumerate() {
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
        let decoded = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(decoded.value, original);
        expected.push(original);
        shared
            .push(index, event.sequence, event.hour, event.variant, decoded)
            .unwrap();
    }
    assert_eq!(cache.entries.len(), 8);
    assert!(Arc::ptr_eq(
        &shared.entries[0].value,
        &shared.entries[count - 8].value
    ));
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows { len: count };
    captured.snapshots = shared;
    let mut owned = Captured::vacant();
    owned.outcome = captured.outcome;
    owned.rows = captured
        .history()
        .iter()
        .zip(&expected)
        .map(|(change, snapshot)| rows::VariantChangeRow {
            snapshot: Some(snapshot.clone()),
            ..change.unwrap().try_clone().unwrap()
        })
        .collect();
    for keyframe in [false, true] {
        for existing in [false, true] {
            assert_eq!(
                converted_bytes(&captured, keyframe, existing),
                converted_bytes(&owned, keyframe, existing)
            );
        }
    }
    let mut output = Vec::new();
    captured.snapshots.materialize(&mut output).unwrap();
    for (row, expected) in output.iter().zip(expected) {
        let expected = rows::VariantChangeRow {
            snapshot: Some(expected),
            ..row.clone()
        };
        let mut actual_bytes = Vec::new();
        let mut expected_bytes = Vec::new();
        rows::encode_row(row, 0, &mut actual_bytes).unwrap();
        rows::encode_row(&expected, 0, &mut expected_bytes).unwrap();
        assert_eq!(actual_bytes, expected_bytes);
    }
}

#[test]
fn a_requested_witness_preserves_failures_bulk_admission_and_read_bytes() {
    let mut log = RawLog::<1>::new(16, 256);
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    let changed = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 9],
    };
    let meaningful = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 9, 3, 4],
    };
    log.take_bulk(&memory, 0x1000, 4, 1).unwrap();
    log.take_bulk(&changed, 0x1000, 4, 1).unwrap();
    log.take_bulk(&meaningful, 0x1000, 4, 1).unwrap();
    let recorded = crate::raw::Recorded::build_range(&log, 0..1).unwrap();
    let observed = witness::Observed::new(&recorded, 2);
    let mut bytes = [0; 2];
    assert!(observed.copy(0x1000, &mut bytes));
    let witness = observed.finish(&log, 0..1, &[0]).unwrap();
    assert!(witness.equal(&log, 0..1, 1..2));
    assert!(!witness.equal(&log, 0..1, 2..3));
    drop(recorded);
    log.spans[1].bulk = false;
    assert!(!witness.equal(&log, 0..1, 1..2));
    log.spans[1].bulk = true;
    log.spans[1].ok = false;
    assert!(!witness.equal(&log, 0..1, 1..2));
    let recorded = crate::raw::Recorded::build_range(&log, 0..1).unwrap();
    let observed = witness::Observed::new(&recorded, 2);
    assert!(!observed.copy(0x2000, &mut bytes));
    assert!(observed.finish(&log, 0..1, &[0]).is_none());
}

#[test]
fn a_requested_witness_still_rejects_contradictory_overlapping_copies() {
    let mut log = RawLog::<1>::new(8, 64);
    let root = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    let nested = engine_native::memory::SliceMemory {
        base: 0x1001,
        bytes: &[2, 3],
    };
    let changed = engine_native::memory::SliceMemory {
        base: 0x1001,
        bytes: &[2, 9],
    };
    log.take_bulk(&root, 0x1000, 4, 1).unwrap();
    log.take_bulk(&nested, 0x1001, 2, 1).unwrap();
    log.take_bulk(&root, 0x1000, 4, 1).unwrap();
    log.take_bulk(&changed, 0x1001, 2, 1).unwrap();
    let mut cache = Cache::default();
    assert!(cache.consistent(&log, 0..2).unwrap());
    let recorded = crate::raw::Recorded::build_range(&log, 0..2).unwrap();
    let observed = witness::Observed::new(&recorded, 2);
    assert!(observed.copy(0x1000, &mut [0; 1]));
    let witness = observed.finish(&log, 0..2, &cache.order).unwrap();
    assert!(!witness.equal(&log, 0..2, 2..4));
    assert!(!cache.consistent(&log, 2..4).unwrap());
}

#[test]
fn unavailable_observation_storage_disables_reuse_without_losing_the_read() {
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    let observed =
        collector_test_support::with_allocations_denied(|| witness::Observed::new(&memory, 1));
    let mut output = [0; 4];
    assert!(collector_test_support::with_allocations_denied(
        || observed.copy(0x1000, &mut output)
    ));
    assert_eq!(output, [1, 2, 3, 4]);
    assert!(
        observed
            .finish(&RawLog::<1>::new(0, 0), 0..0, &[])
            .is_none()
    );
}

#[test]
fn unavailable_witness_storage_does_not_refuse_a_successful_observation() {
    let mut log = RawLog::<1>::new(1, 4);
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    log.take_bulk(&memory, 0x1000, 4, 1).unwrap();
    let recorded = crate::raw::Recorded::build_range(&log, 0..1).unwrap();
    let observed = witness::Observed::new(&recorded, 1);
    let mut output = [0; 2];
    assert!(observed.copy(0x1000, &mut output));
    assert!(
        collector_test_support::with_allocations_denied(|| { observed.finish(&log, 0..1, &[0]) })
            .is_none()
    );
    assert_eq!(output, [1, 2]);
}

#[test]
fn optional_cache_failures_never_reach_the_terminal_hook_but_required_ones_still_do() {
    thread_local! {
        static TERMINAL: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    // Counted per thread: oom() calls the hook on the failing thread, so parallel tests can't interfere.
    fn terminal() {
        TERMINAL.with(|calls| calls.set(calls.get() + 1));
    }
    struct Hooks;
    impl Drop for Hooks {
        fn drop(&mut self) {
            engine_allocation::set_hooks(None, None);
        }
    }
    let batch = burst(2);
    let event = &batch.events[0];
    let range = event.start..event.end;
    let value = Cache::default()
        .decode(&batch, event, 0, 7, &mut Shared::default())
        .unwrap();
    let mut log = RawLog::<1>::new(1, 4);
    let bytes = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    log.take_bulk(&bytes, 0x1000, 4, 1).unwrap();
    let memory = crate::raw::Recorded::build_range(&log, 0..1).unwrap();
    let read = witness::Observed::new(&memory, 1);
    let mut output = [0; 2];
    assert!(read.copy(0x1000, &mut output));
    let witnessed = witness::Observed::new(&bytes, 1);
    assert!(witnessed.copy(0x1000, &mut output));
    let read_witness = witnessed.finish(&log, 0..1, &[0]);
    assert!(read_witness.is_some());
    let denials = || collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed);
    let mut cache = Cache::default();
    let hooks = Hooks;
    engine_allocation::set_hooks(None, Some(terminal));
    assert_eq!(
        collector_test_support::with_allocations_denied(
            || cache.consistent(&batch.log, range.clone())
        ),
        Ok(false)
    );
    assert!(!collector_test_support::with_allocations_denied(|| {
        cache.insert(&batch.log, range.clone(), event.pointer, 0, &value, None)
    }));
    collector_test_support::with_allocations_denied(|| cache.remember(event.pointer, 0));
    let unobserved =
        collector_test_support::with_allocations_denied(|| witness::Observed::new(&memory, 1));
    assert!(unobserved.finish(&log, 0..1, &[0]).is_none());
    assert!(
        collector_test_support::with_allocations_denied(|| read.finish(&log, 0..1, &[0])).is_none()
    );
    // Reserving heads succeeds, entries fails: the partial reservation leaves no entry behind.
    let partial = {
        let _guard = collector_test_support::NullGuard::new(2);
        cache.insert(&batch.log, range.clone(), event.pointer, 0, &value, None)
    };
    assert!(!partial);
    assert!(cache.entries.is_empty() && cache.heads.is_empty() && cache.recent.is_empty());
    // Read-list growth past its initial room: the first denied growth stops recording reads.
    let growing = witness::Observed::new(&bytes, 1);
    let grown = collector_test_support::with_allocations_denied(|| {
        for _ in 0..16 {
            assert!(growing.copy(0x1000, &mut output));
        }
        denials()
    });
    assert_eq!(grown, 1);
    assert!(growing.finish(&log, 0..1, &[0]).is_none());
    // Heads and entries succeed, the witness index and the recent map are denied.
    let mut indexed = Cache::default();
    let (inserted, indexed_denials) = {
        let _guard = collector_test_support::NullGuard::new(3);
        let inserted = indexed.insert(&log, 0..1, event.pointer, 0, &value, read_witness);
        (inserted, denials())
    };
    assert!(inserted);
    assert_eq!(indexed_denials, 2);
    assert_eq!(indexed.entries.len(), 1);
    assert!(indexed.entries[0].witness.is_none());
    assert!(indexed.witness_heads.is_empty() && indexed.recent.is_empty());
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    let mut shared = Shared::default();
    assert!(collector_test_support::with_allocations_denied(|| shared.reserve(1)).is_err());
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 1);
    drop(hooks);
    let fresh = cache
        .decode(&batch, event, 0, 7, &mut Shared::default())
        .unwrap();
    assert_eq!(fresh.value, value.value);
    assert!(!Arc::ptr_eq(&fresh, &value));
}
