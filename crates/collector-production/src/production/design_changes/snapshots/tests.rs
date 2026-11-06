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
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 8);
    assert_eq!(
        cache.counts[CacheOutcome::ReadWitnessHit as usize],
        count as u64 - 8
    );
    assert_eq!(cache.counts[CacheOutcome::FullCopyHit as usize], 0);
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
    let witness = observed.finish(&log, 0..1, &[0], false).unwrap();
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
    assert!(observed.finish(&log, 0..1, &[0], false).is_none());
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
    let witness = observed.finish(&log, 0..2, &cache.order, false).unwrap();
    assert!(!witness.equal(&log, 0..2, 2..4));
    assert_eq!(
        witness.mismatch(&log, 0..2, 2..4),
        Some(CacheOutcome::Overlap)
    );
    assert!(!cache.consistent(&log, 2..4).unwrap());
    assert_eq!(cache.counts[CacheOutcome::InsertOverlap as usize], 1);
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
            .finish(&RawLog::<1>::new(0, 0), 0..0, &[], false)
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
        collector_test_support::with_allocations_denied(|| {
            observed.finish(&log, 0..1, &[0], false)
        })
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
    let read_witness = witnessed.finish(&log, 0..1, &[0], false);
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
    collector_test_support::with_allocations_denied(|| {
        cache.remember(event.pointer, 0, range.clone())
    });
    let unobserved =
        collector_test_support::with_allocations_denied(|| witness::Observed::new(&memory, 1));
    assert!(unobserved.finish(&log, 0..1, &[0], false).is_none());
    assert!(
        collector_test_support::with_allocations_denied(|| read.finish(&log, 0..1, &[0], false))
            .is_none()
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
    assert!(growing.finish(&log, 0..1, &[0], false).is_none());
    // Heads and entries succeed; witness indexing, source candidates and the recent map fail.
    let mut indexed = Cache::default();
    let (inserted, indexed_denials) = {
        let _guard = collector_test_support::NullGuard::new(3);
        let inserted = indexed.insert(&log, 0..1, event.pointer, 0, &value, read_witness);
        (inserted, denials())
    };
    assert!(inserted);
    assert_eq!(indexed_denials, 3);
    assert_eq!(indexed.entries.len(), 1);
    assert!(indexed.entries[0].witness.is_none());
    assert!(
        indexed.witness_heads.is_empty()
            && indexed.source_heads.is_empty()
            && indexed.recent.is_empty()
    );
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    assert_eq!(cache.counts[CacheOutcome::AllocationSkipped as usize], 3);
    assert_eq!(indexed.counts[CacheOutcome::AllocationSkipped as usize], 3);
    // The read index reserves, but the shape index fails: neither optional index publishes.
    let observed = witness::Observed::new(&memory, 1);
    assert!(observed.copy(0x1000, &mut output));
    let witness = observed.finish(&log, 0..1, &[0], false);
    assert!(witness.is_some());
    let mut shaped = Cache::default();
    let (inserted, denied) = {
        let _guard = collector_test_support::NullGuard::new(4);
        let inserted = shaped.insert(&log, 0..1, event.pointer, 0, &value, witness);
        (inserted, denials())
    };
    assert!(inserted);
    assert_eq!(denied, 3);
    assert!(shaped.entries[0].witness.is_none());
    assert!(
        shaped.witness_heads.is_empty()
            && shaped.shape_heads.is_empty()
            && shaped.source_heads.is_empty()
            && shaped.recent.is_empty()
    );
    assert_eq!(shaped.counts[CacheOutcome::AllocationSkipped as usize], 3);
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    for ordinal in [3, 4] {
        let mut candidate = Cache::default();
        engine_allocation::reserve_map(&mut candidate.recent, 1).unwrap();
        let (inserted, denied) = {
            let _guard = collector_test_support::NullGuard::new(ordinal);
            let inserted =
                candidate.insert(&batch.log, range.clone(), event.pointer, 0, &value, None);
            (inserted, denials())
        };
        assert!(inserted);
        assert_eq!(denied, 1);
        assert_eq!(candidate.entries.len(), 1);
        assert_eq!(candidate.source_heads.is_empty(), ordinal == 3);
        if ordinal == 4 {
            let heads = &candidate.source_heads[&value.definition.identity.id];
            assert_eq!(heads.pointer, event.pointer);
            assert!(heads.states.is_empty());
        }
        assert_eq!(candidate.recent[&event.pointer], (0, None));
        assert_eq!(
            candidate.counts[CacheOutcome::AllocationSkipped as usize],
            1
        );
        assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
        candidate.remember(event.pointer, 0, range.clone());
        let head = &candidate.source_heads[&value.definition.identity.id].states
            [&(value.definition.design.bonus_sources.len() as u32)];
        assert_eq!(head.index, Some(0));
        assert_eq!(head.range, range);
        assert!(Arc::ptr_eq(&head.value, &value));
        assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    }
    mission_proof_denials_are_nonterminal();
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

#[test]
fn cache_diagnostics_partition_full_decodes_and_do_not_change_reused_payloads() {
    let mut batch = burst(3);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let first = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    let repeated = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &repeated));
    assert_eq!(cache.counts[CacheOutcome::FullCopyHit as usize], 1);
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 1);
    assert_eq!(cache.counts[CacheOutcome::NoRecentEntry as usize], 1);
    let changed = cache
        .decode(&batch, &batch.events[1], 0, 7, &mut shared)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(cache.counts[CacheOutcome::ReadBytes as usize], 1);
    let shape = batch.events[2].start;
    batch.log.spans[shape].addr += 1;
    let left = &batch.log.spans[batch.events[0].start..batch.events[0].end];
    let right = &batch.log.spans[batch.events[2].start..batch.events[2].end];
    assert_eq!(
        witness::shape_miss(left, right),
        Some(CacheOutcome::SpanAddress)
    );
    assert_eq!(
        witness::shape_miss(left, &right[..right.len() - 1]),
        Some(CacheOutcome::SpanCount)
    );
    batch.log.spans[shape].addr -= 1;
    batch.log.spans[shape].bulk = !batch.log.spans[shape].bulk;
    let left = &batch.log.spans[batch.events[0].start..batch.events[0].end];
    let right = &batch.log.spans[batch.events[2].start..batch.events[2].end];
    assert_eq!(
        witness::shape_miss(left, right),
        Some(CacheOutcome::SpanMetadata)
    );
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 2);
}

fn changing_source_shapes(count: usize) -> Batch {
    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let start = address - kit::BASE;
        bytes[start..start + value.len()].copy_from_slice(value);
    }
    let mut bytes = fixture();
    let mut pointers = Vec::new();
    for value in 0..3_i64 {
        let pointer = kit::BASE + bytes.len();
        pointers.push(pointer as u64);
        bytes.resize(bytes.len() + native::BONUS_IDEA, 0);
        put(
            &mut bytes,
            pointer,
            &(native::SPECIFIC_BONUS_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        let stats = kit::BASE + bytes.len();
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        put(
            &mut bytes,
            pointer + native::BONUS_STATS,
            &(stats as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + native::BONUS_STATS + 8,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + native::BONUS_STATS + 12,
            &1_u32.to_le_bytes(),
        );
    }
    let sources = kit::BASE + bytes.len();
    for pointer in pointers {
        bytes.extend_from_slice(&pointer.to_le_bytes());
    }
    put(
        &mut bytes,
        kit::VARIANT + native::BONUS_SOURCES,
        &(sources as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        kit::VARIANT + native::BONUS_SOURCES + 8,
        &3_u32.to_le_bytes(),
    );
    let mut batch = Batch::empty();
    batch.prepare(count, count * 128, count * ROOT_BYTES * 4);
    for index in 0..count {
        put(
            &mut bytes,
            kit::VARIANT + native::BONUS_SOURCES + 12,
            &((index % 4) as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            kit::VARIANT + native::UPGRADES + 24,
            &(index as u64).to_le_bytes(),
        );
        append(&mut batch, &bytes, index as u64 + 1, false);
    }
    batch
}

#[test]
fn older_source_list_shapes_reuse_only_their_exact_reads_and_keep_every_row() {
    let batch = changing_source_shapes(4_096);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut owned = Captured::vacant();
    for (index, event) in batch.events.iter().enumerate() {
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
        let decoded = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(decoded.value, original);
        assert_eq!(decoded.definition.design.bonus_sources.len(), index % 4);
        owned.rows.push(rows::VariantChangeRow {
            sequence: event.sequence,
            hour: event.hour,
            variant: event.variant,
            snapshot: Some(original),
            ..Default::default()
        });
        shared
            .push(index, event.sequence, event.hour, event.variant, decoded)
            .unwrap();
    }
    assert_eq!(cache.entries.len(), 4);
    assert_eq!(cache.shape_heads.len(), 4);
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 4);
    assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 4_092);
    assert_eq!(cache.counts[CacheOutcome::FullCopyHit as usize], 0);
    for index in 4..shared.len() {
        assert!(Arc::ptr_eq(
            &shared.entries[index].value,
            &shared.entries[index % 4].value
        ));
    }
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows {
        len: batch.events.len(),
    };
    captured.snapshots = shared;
    owned.outcome = captured.outcome;
    for keyframe in [false, true] {
        for existing in [false, true] {
            assert_eq!(
                converted_bytes(&captured, keyframe, existing),
                converted_bytes(&owned, keyframe, existing)
            );
        }
    }
}

#[test]
fn a_colliding_shape_candidate_cannot_substitute_another_shape_or_changed_read_bytes() {
    let mut batch = changing_source_shapes(8);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    for event in &batch.events[..4] {
        cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
    }
    cache.source_heads.clear();
    let event = batch.events[4];
    let key = shape_key(&batch.log, event.start..event.end, event.pointer);
    cache.shape_heads.insert(key, 3);
    let value = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert!(value.definition.design.bonus_sources.is_empty());
    assert!(!Arc::ptr_eq(&value, &cache.entries[3].value));
    let event = batch.events[5];
    let root = &batch.log.spans[event.start];
    let at = root.start as usize + native::DISPLAY_NAME;
    batch.log.bytes[at] = b'X';
    batch.log.bytes[at + 16..at + 24].copy_from_slice(&1_u64.to_le_bytes());
    let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
    let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
    let value = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert_eq!(value.value, original);
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 6);
}

#[test]
fn equal_rows_with_different_read_capacities_keep_distinct_witnesses_for_each_shape() {
    let mut batch = changing_source_shapes(60);
    for (index, event) in batch.events.iter().enumerate() {
        let at = batch.log.spans[event.start].start as usize + native::BONUS_SOURCES + 8;
        batch.log.bytes[at..at + 4].copy_from_slice(&(3 + (index / 4 % 3) as u32).to_le_bytes());
    }
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut values = Vec::new();
    for event in &batch.events {
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
        let value = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(value.value, original);
        values.push(value);
    }
    assert_eq!(cache.shape_heads.len(), 4);
    assert_eq!(cache.entries.len(), 12);
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 12);
    assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 48);
    assert_eq!(values[0].value, values[4].value);
    assert!(!Arc::ptr_eq(&values[0], &values[4]));
    assert!(Arc::ptr_eq(&values[0], &values[12]));
}

fn relocated_mission_shapes(count: usize) -> Batch {
    let source = changing_source_shapes(count);
    let mut batch = Batch::empty();
    batch.prepare(count, count * 160, count * ROOT_BYTES * 6);
    for (index, event) in source.events.iter().enumerate() {
        let memory =
            crate::raw::Recorded::build_range(&source.log, event.start..event.end).unwrap();
        let mut log = RawLog::<1>::new(160, ROOT_BYTES * 6);
        for span in &source.log.spans[event.start..event.end] {
            log.take_bulk(&memory, span.addr, span.len as usize, 1)
                .unwrap();
        }
        let outer = 0x100_0000 + index * 0x1000;
        let stats = outer + 0x100;
        let mut bytes = [0; 128];
        bytes[..4].copy_from_slice(&3_u32.to_le_bytes());
        bytes[16..24].copy_from_slice(&(stats as u64).to_le_bytes());
        bytes[24..28].copy_from_slice(&2_u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&2_u32.to_le_bytes());
        let root = log
            .spans
            .iter()
            .find(|span| span.addr == kit::VARIANT + native::MISSION_GROUP_DATA)
            .unwrap()
            .start as usize;
        log.bytes[root..root + 8].copy_from_slice(&(outer as u64).to_le_bytes());
        log.bytes[root + 8..root + 12].copy_from_slice(&1_u32.to_le_bytes());
        log.bytes[root + 12..root + 16].copy_from_slice(&1_u32.to_le_bytes());
        let outer_memory = engine_native::memory::SliceMemory {
            base: outer,
            bytes: &bytes,
        };
        log.take_bulk(&outer_memory, outer, 40, 1).unwrap();
        let mut bytes = [0; 32];
        bytes[..4].copy_from_slice(&9_u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&17_i64.to_le_bytes());
        bytes[16..20].copy_from_slice(&12_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&(-23_i64).to_le_bytes());
        let stats_memory = engine_native::memory::SliceMemory {
            base: stats,
            bytes: &bytes,
        };
        log.take_bulk(&stats_memory, stats, bytes.len(), 1).unwrap();
        let (start, end) = storage::append(&mut batch.log, &log).unwrap();
        batch.events.push(Event {
            start,
            end,
            ..*event
        });
    }
    batch
}

fn replace_copied_bytes(batch: &mut Batch, event: &Event, address: usize, value: &[u8]) {
    let mut copies = 0;
    for span in &batch.log.spans[event.start..event.end] {
        if span.addr <= address && address + value.len() <= span.addr + span.len as usize {
            let start = span.start as usize + address - span.addr;
            batch.log.bytes[start..start + value.len()].copy_from_slice(value);
            copies += 1;
        }
    }
    assert_ne!(copies, 0);
}

#[test]
fn distinct_nonmission_states_keep_every_candidate_chain_and_mission_read_count_bounded() {
    use std::cell::Cell;
    struct Counted<'a, M> {
        source: &'a M,
        reads: Cell<usize>,
    }
    impl<M: Memory> Memory for Counted<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            self.source.copy(address, output)
        }
    }
    const STATES: usize = 128;
    for missions in [false, true] {
        for initially_relocated in [false, true] {
            let mut batch = if missions {
                relocated_mission_shapes(STATES * 2)
            } else {
                changing_source_shapes(STATES * 2)
            };
            for index in 0..batch.events.len() {
                let event = batch.events[index];
                replace_copied_bytes(
                    &mut batch,
                    &event,
                    event.pointer + native::BONUS_SOURCES + 12,
                    &0_u32.to_le_bytes(),
                );
                replace_copied_bytes(
                    &mut batch,
                    &event,
                    event.pointer + native::MAP_RANGE_STAT,
                    &((index % STATES) as i64).to_le_bytes(),
                );
                replace_copied_bytes(
                    &mut batch,
                    &event,
                    event.pointer + native::CAN_UPGRADE_MODULES,
                    &[(index % 2) as u8],
                );
                if missions && !initially_relocated && index < STATES {
                    let outer = 0x100_0000 + index * 0x1000;
                    let fixed = 0x100_0000;
                    replace_copied_bytes(
                        &mut batch,
                        &event,
                        event.pointer + native::MISSION_GROUP_DATA,
                        &(fixed as u64).to_le_bytes(),
                    );
                    replace_copied_bytes(
                        &mut batch,
                        &event,
                        outer + 16,
                        &((fixed + 0x100) as u64).to_le_bytes(),
                    );
                    for span in &mut batch.log.spans[event.start..event.end] {
                        if span.addr == outer || span.addr == outer + 0x100 {
                            span.addr -= outer - fixed;
                        }
                    }
                }
            }
            let mut cache = Cache::default();
            let mut shared = Shared::default();
            let mut originals = Vec::new();
            let mut model_keys = [None; 4];
            let mut miss_reads = [None; 4];
            let mut hit_reads = [None; 4];
            for (index, event) in batch.events.iter().enumerate() {
                let recorded =
                    crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
                let original = snapshot(&recorded, 0, 7, event.pointer, false).unwrap();
                if missions {
                    let current_key = missions::model_key(
                        &recorded,
                        event.pointer,
                        &batch.log.spans[event.start..event.end],
                    )
                    .unwrap();
                    assert_eq!(
                        *model_keys[index % 4].get_or_insert(current_key),
                        current_key
                    );
                    let counted = Counted {
                        source: &recorded,
                        reads: Cell::new(0),
                    };
                    let probes = cache.probes;
                    let found = engine_allocation::advisory(|| {
                        cache.mission_candidate(
                            &counted,
                            event.pointer,
                            &batch.log.spans[event.start..event.end],
                        )
                    });
                    let hit = index >= STATES;
                    assert_eq!(found, hit.then_some(index % STATES));
                    assert_eq!(cache.probes[2] - probes[2], usize::from(hit));
                    if index < 4 {
                        assert_eq!(counted.reads.get(), 6);
                    } else {
                        let reads = if hit {
                            &mut hit_reads[index % 4]
                        } else {
                            &mut miss_reads[index % 4]
                        };
                        assert_eq!(
                            *reads.get_or_insert(counted.reads.get()),
                            counted.reads.get()
                        );
                    }
                }
                let probes = cache.probes;
                let value = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
                for (before, after) in probes.into_iter().zip(cache.probes) {
                    assert!(after - before <= 1, "candidate chain grew at state {index}");
                }
                assert_eq!(value.value, original);
                if index < STATES {
                    originals.push(value);
                    assert_eq!(cache.entries.len(), index + 1);
                } else {
                    assert!(Arc::ptr_eq(&value, &originals[index % STATES]));
                    assert_eq!(cache.entries.len(), STATES);
                }
            }
            assert_eq!(
                cache.counts[CacheOutcome::FullDecode as usize],
                STATES as u64
            );
            assert_eq!(
                cache.counts[CacheOutcome::ReadWitnessHit as usize],
                STATES as u64
            );
            if missions {
                assert_eq!(cache.mission_models.len(), 4);
                assert_eq!(cache.mission_heads.len(), STATES);
                for (hit, miss) in hit_reads.into_iter().zip(miss_reads) {
                    assert!(hit.unwrap() > miss.unwrap());
                }
            }
        }
    }
}

#[test]
fn relocated_nonmission_buffers_select_only_their_own_mission_read_plans() {
    fn key_reads(cache: &Cache<'_>) -> usize {
        cache
            .entries
            .iter()
            .filter_map(|entry| entry.missions.as_ref())
            .map(|proof| proof.key_reads.get())
            .sum()
    }
    const STATES: usize = 128;
    let mut source = relocated_mission_shapes(STATES * 2);
    let mut batch = Batch::empty();
    batch.prepare(STATES * 2, STATES * 2 * 160, STATES * 2 * ROOT_BYTES * 6);
    for index in 0..source.events.len() {
        let event = source.events[index];
        let resources = 0x200_0000 + (index % STATES) * 0x1000;
        let mut header = [0; 16];
        header[..8].copy_from_slice(&(resources as u64).to_le_bytes());
        header[8..12].copy_from_slice(&1_u32.to_le_bytes());
        header[12..].copy_from_slice(&1_u32.to_le_bytes());
        replace_copied_bytes(
            &mut source,
            &event,
            event.pointer + native::BUILD_RESOURCES,
            &header,
        );
        replace_copied_bytes(
            &mut source,
            &event,
            event.pointer + native::BONUS_SOURCES + 12,
            &0_u32.to_le_bytes(),
        );
        let recorded =
            crate::raw::Recorded::build_range(&source.log, event.start..event.end).unwrap();
        let mut log = RawLog::<1>::new(160, ROOT_BYTES * 6);
        for span in &source.log.spans[event.start..event.end] {
            log.take_bulk(&recorded, span.addr, span.len as usize, 1)
                .unwrap();
        }
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&17_i64.to_le_bytes());
        bytes[8..12].copy_from_slice(&9_u32.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
            base: resources,
            bytes: &bytes,
        };
        log.take_bulk(&memory, resources, bytes.len(), 1).unwrap();
        let (start, end) = storage::append(&mut batch.log, &log).unwrap();
        batch.events.push(Event {
            start,
            end,
            ..event
        });
    }
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut originals = Vec::new();
    let mut reads_per_key = Vec::new();
    for (index, event) in batch.events.iter().enumerate() {
        let recorded =
            crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let state = index % STATES;
        let hit = index >= STATES;
        let visits = cache.mission_model_visits;
        let reads = key_reads(&cache);
        let candidate = cache.mission_candidate(
            &recorded,
            event.pointer,
            &batch.log.spans[event.start..event.end],
        );
        assert_eq!(candidate, hit.then_some(state));
        assert_eq!(cache.mission_model_visits[0] - visits[0], usize::from(hit));
        assert_eq!(cache.mission_model_visits[1], visits[1]);
        assert_eq!(
            key_reads(&cache) - reads,
            if hit { reads_per_key[state] } else { 0 },
        );
        let visits = cache.mission_model_visits;
        let reads = key_reads(&cache);
        let value = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(cache.mission_model_visits[0] - visits[0], usize::from(hit));
        assert_eq!(cache.mission_model_visits[1], visits[1]);
        let original = snapshot(&recorded, 0, 7, event.pointer, false).unwrap();
        assert_eq!(value.value, original);
        assert_eq!(value.definition.design.build_resources[0].value, 17);
        if hit {
            assert_eq!(key_reads(&cache) - reads, reads_per_key[state]);
            assert!(Arc::ptr_eq(&value, &originals[state]));
            assert_eq!(cache.entries.len(), STATES);
        } else {
            let reads = key_reads(&cache) - reads;
            assert_ne!(reads, 0);
            reads_per_key.push(reads);
            assert!(originals.iter().all(|other| !Arc::ptr_eq(&value, other)));
            originals.push(value);
            assert_eq!(cache.entries.len(), index + 1);
        }
    }
    assert_eq!(cache.mission_models.len(), STATES);
    assert_eq!(cache.mission_heads.len(), STATES);
    for &head in cache.mission_models.values() {
        assert!(cache.entries[head].mission_model_next.is_none());
    }
    assert_eq!(
        cache.counts[CacheOutcome::FullDecode as usize],
        STATES as u64
    );
    assert_eq!(
        cache.counts[CacheOutcome::ReadWitnessHit as usize],
        STATES as u64
    );
}

#[test]
fn cold_context_read_plans_keep_their_original_states_after_warm_models_arrive() {
    const STATES: usize = 8;
    for missions in [false, true] {
        let mut batch = if missions {
            relocated_mission_shapes(STATES * 2)
        } else {
            changing_source_shapes(STATES * 2)
        };
        for index in 0..batch.events.len() {
            let event = batch.events[index];
            replace_copied_bytes(
                &mut batch,
                &event,
                event.pointer + native::BONUS_SOURCES + 12,
                &0_u32.to_le_bytes(),
            );
            replace_copied_bytes(
                &mut batch,
                &event,
                event.pointer + native::MAP_RANGE_STAT,
                &((index % STATES) as i64).to_le_bytes(),
            );
        }
        let event = &batch.events[0];
        let range = event.start..event.end;
        let recorded = crate::raw::Recorded::build_range(&batch.log, range.clone()).unwrap();
        snapshot(&recorded, 0, 7, event.pointer, false).unwrap();
        let observed = witness::Observed::new(&recorded, range.len());
        let first = Arc::new(decode_snapshot(&observed, 0, 7, event.pointer, false).unwrap());
        // Make the cold-only lookup explicit, independent of the shared content cache's warmth.
        assert!(observed.copy(
            kit::EQUIPMENT_TYPE + engine_native::cequipment_type::DESIGN_COST_MODIFIER,
            &mut [0; 4],
        ));
        let mut cache = Cache::default();
        assert!(cache.consistent(&batch.log, range.clone()).unwrap());
        let witness = observed
            .finish(&batch.log, range.clone(), &cache.order, true)
            .unwrap();
        let proof = missions::Proof::build(
            &recorded,
            &witness,
            event.pointer,
            &first.definition.design.mission_stats,
        );
        assert!(cache.insert(
            &batch.log,
            range.clone(),
            event.pointer,
            fingerprint(&batch.log, range, event.pointer),
            &first,
            Some(witness),
        ));
        cache.index_missions(
            &recorded,
            event.pointer,
            proof,
            &batch.log.spans[event.start..event.end],
        );
        let mut originals = vec![first];
        let mut shared = Shared::default();
        for (index, event) in batch.events.iter().enumerate().skip(1) {
            if missions && index >= STATES {
                let recorded =
                    crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
                assert_eq!(
                    cache.mission_candidate(
                        &recorded,
                        event.pointer,
                        &batch.log.spans[event.start..event.end],
                    ),
                    Some(index % STATES)
                );
            }
            let probes = cache.probes;
            let value = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
            for (before, after) in probes.into_iter().zip(cache.probes) {
                assert!(after - before <= 1);
            }
            if index < STATES {
                originals.push(value);
            } else {
                assert!(Arc::ptr_eq(&value, &originals[index % STATES]));
            }
        }
        assert_eq!(cache.entries.len(), STATES);
        assert_eq!(
            cache.counts[CacheOutcome::FullDecode as usize],
            (STATES - 1) as u64
        );
        assert_eq!(
            cache.counts[CacheOutcome::ReadWitnessHit as usize],
            STATES as u64
        );
        for &head in cache.shape_heads.values() {
            assert!(
                std::iter::successors(Some(head), |&index| cache.entries[index].shape_next).count()
                    <= 2
            );
        }
        if missions {
            assert_eq!(cache.mission_models.len(), 4);
            let mut plans = cache
                .mission_models
                .values()
                .map(|&head| {
                    std::iter::successors(Some(head), |&index| {
                        cache.entries[index].mission_model_next
                    })
                    .count()
                })
                .collect::<Vec<_>>();
            plans.sort_unstable();
            assert_eq!(plans, [1, 1, 1, 2]);
        }
    }
}

#[test]
fn rebuilt_mission_storage_reuses_exact_states_across_every_distinct_layout() {
    let batch = relocated_mission_shapes(512);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let mut owned = Captured::vacant();
    for (index, event) in batch.events.iter().enumerate() {
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
        let value = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(value.value, original);
        owned.rows.push(rows::VariantChangeRow {
            sequence: event.sequence,
            hour: event.hour,
            variant: event.variant,
            snapshot: Some(original),
            ..Default::default()
        });
        shared
            .push(index, event.sequence, event.hour, event.variant, value)
            .unwrap();
    }
    assert_eq!(cache.entries.len(), 4);
    assert!(cache.entries.iter().all(|entry| entry.missions.is_some()));
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 4);
    assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 508);
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows {
        len: batch.events.len(),
    };
    captured.snapshots = shared;
    owned.outcome = captured.outcome;
    for keyframe in [false, true] {
        for existing in [false, true] {
            assert_eq!(
                converted_bytes(&captured, keyframe, existing),
                converted_bytes(&owned, keyframe, existing)
            );
        }
    }
}

#[test]
fn mission_layout_reuse_checks_masks_stats_invariants_admission_and_all_overlaps() {
    let mut batch = relocated_mission_shapes(12);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    let first = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    let originals = std::iter::once(Arc::clone(&first))
        .chain(
            batch.events[1..4]
                .iter()
                .map(|event| cache.decode(&batch, event, 0, 7, &mut shared).unwrap()),
        )
        .collect::<Vec<_>>();
    for (ordinal, event) in batch.events[4..7].iter().enumerate() {
        let span = if ordinal == 0 {
            &batch.log.spans[event.end - 2]
        } else if ordinal == 1 {
            &batch.log.spans[event.end - 1]
        } else {
            &batch.log.spans[event.start]
        };
        let offset = [0, 8, native::TYPE][ordinal];
        batch.log.bytes[span.start as usize + offset] ^= 1;
    }
    for event in &batch.events[4..6] {
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
        let original = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
        let decoded = cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
        assert_eq!(decoded.value, original);
        assert!(
            originals
                .iter()
                .all(|original| !Arc::ptr_eq(original, &decoded))
        );
    }
    assert!(
        cache
            .decode(&batch, &batch.events[6], 0, 7, &mut shared)
            .is_err()
    );
    let event = batch.events[7];
    let span = batch.log.spans[event.start..event.end]
        .iter()
        .find(|span| span.addr == event.pointer + native::MAP_RANGE_STAT)
        .unwrap();
    batch.log.bytes[span.start as usize] ^= 1;
    let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
    let expected = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
    drop(memory);
    let decoded = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert_eq!(decoded.value, expected);
    assert!(
        originals
            .iter()
            .all(|original| !Arc::ptr_eq(original, &decoded))
    );
    let event = batch.events[8];
    batch.log.spans[event.end - 1].bulk = false;
    assert!(cache.decode(&batch, &event, 0, 7, &mut shared).is_err());
    let event = batch.events[9];
    let span = batch.log.spans[event.end - 2];
    batch.log.bytes[span.start as usize + 24..span.start as usize + 28]
        .copy_from_slice(&0_u32.to_le_bytes());
    assert!(cache.decode(&batch, &event, 0, 7, &mut shared).is_err());
    let mut event = batch.events[10];
    let span = batch.log.spans[event.end - 1];
    let conflicting = engine_native::memory::SliceMemory {
        base: span.addr,
        bytes: &[0; 32],
    };
    let at = batch.log.spans.len();
    batch.log.take_bulk(&conflicting, span.addr, 32, 1).unwrap();
    let mut spans = batch.log.spans[event.start..event.end].to_vec();
    spans.push(batch.log.spans[at]);
    event.start = batch.log.spans.len();
    batch.log.spans.extend_from_slice(&spans);
    event.end = batch.log.spans.len();
    if let Ok(decoded) = cache.decode(&batch, &event, 0, 7, &mut shared) {
        assert!(
            originals
                .iter()
                .all(|original| !Arc::ptr_eq(original, &decoded))
        );
    }
}

#[test]
fn mission_proofs_refuse_aliased_invariant_reads_and_optional_failures() {
    let batch = relocated_mission_shapes(1);
    let event = batch.events[0];
    let recorded = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
    let observed = witness::Observed::new(&recorded, event.end - event.start);
    let value = decode_snapshot(&observed, 0, 7, event.pointer, false).unwrap();
    let span = batch.log.spans[event.end - 1];
    assert!(observed.copy(span.addr, &mut [0; 16]));
    let mut cache = Cache::default();
    assert!(
        cache
            .consistent(&batch.log, event.start..event.end)
            .unwrap()
    );
    let witness = observed
        .finish(&batch.log, event.start..event.end, &cache.order, true)
        .unwrap();
    assert!(
        engine_allocation::advisory(|| missions::Proof::build(
            &recorded,
            &witness,
            event.pointer,
            &value.definition.design.mission_stats
        ))
        .is_none()
    );
    assert!(
        collector_test_support::with_allocations_denied(|| engine_allocation::advisory(|| {
            missions::Proof::build(
                &recorded,
                &witness,
                event.pointer,
                &value.definition.design.mission_stats,
            )
        }))
        .is_none()
    );
    let value = cache
        .decode(&batch, &event, 0, 7, &mut Shared::default())
        .unwrap();
    assert_eq!(value.value.definition.id, event.variant);
}

#[test]
fn mission_key_collisions_still_check_invariant_bytes_and_pointer_identity() {
    let mut batch = relocated_mission_shapes(12);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    for event in &batch.events[..4] {
        cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
    }
    let event = batch.events[4];
    let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
    let model_key = missions::model_key(
        &memory,
        event.pointer,
        &batch.log.spans[event.start..event.end],
    )
    .unwrap();
    let model = &cache.entries[cache.mission_models[&model_key]];
    let key = model
        .missions
        .as_ref()
        .unwrap()
        .key(
            &memory,
            model.witness.as_ref().unwrap(),
            model_key,
            &mut Vec::new(),
        )
        .unwrap();
    drop(memory);
    cache.mission_heads.insert(key, 3);
    cache.entries[3].mission_next = Some(0);
    let value = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert!(Arc::ptr_eq(&value, &cache.entries[0].value));
    assert!(!Arc::ptr_eq(&value, &cache.entries[3].value));
    let event = batch.events[8];
    cache.entries[0].pointer += 1;
    let other = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert!(!Arc::ptr_eq(&other, &value));
    assert_eq!(other.value, value.value);
    let event = batch.events[9];
    let span = batch.log.spans[event.end - 1];
    batch.log.bytes[span.start as usize + 4] ^= 1;
    let current = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    assert!(Arc::ptr_eq(&current, &cache.entries[1].value));
}

fn mission_proof_denials_are_nonterminal() {
    let batch = relocated_mission_shapes(5);
    let mut cache = Cache::default();
    let mut shared = Shared::default();
    cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    let entry = &cache.entries[0];
    let proof = entry.missions.as_ref().unwrap();
    let witness = entry.witness.as_ref().unwrap();
    let event = batch.events[4];
    let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end).unwrap();
    assert!(
        collector_test_support::with_allocations_denied(|| engine_allocation::advisory(|| {
            proof.equal(
                &memory,
                witness,
                event.pointer,
                &entry.value.definition.design.mission_stats,
                &mut Vec::new(),
            )
        }))
        .is_none()
    );
    assert!(
        collector_test_support::with_allocations_denied(|| engine_allocation::advisory(|| {
            missions::Proof::build(
                &memory,
                witness,
                event.pointer,
                &entry.value.definition.design.mission_stats,
            )
        }))
        .is_none()
    );
}

#[test]
fn source_count_witnesses_survive_monthly_gaps_without_retaining_design_revisions() {
    let batch = changing_source_shapes(4);
    let mut retained = Retained::default();
    let mut first = Cache::with_retained(&mut retained);
    let mut shared = Shared::default();
    let values: Vec<_> = batch
        .events
        .iter()
        .map(|event| first.decode(&batch, event, 0, 7, &mut shared).unwrap())
        .collect();
    assert_eq!(first.counts[CacheOutcome::FullDecode as usize], 4);
    first.finish(&batch);
    drop(first);
    let mut later = Cache::with_retained(&mut retained);
    for (index, event) in batch.events.iter().enumerate() {
        let actual = later
            .decode(&batch, event, 0, 7 + 744, &mut shared)
            .unwrap();
        assert!(Arc::ptr_eq(&actual, &values[index]));
    }
    assert_eq!(later.counts[CacheOutcome::FullDecode as usize], 0);
    assert_eq!(later.counts[CacheOutcome::ReadWitnessHit as usize], 4);
    later.finish(&batch);
    drop(later);
    assert_eq!(retained.state_count(), 4);
    for revision in 0..32 {
        let mut changed = changing_source_shapes(4);
        for event in &changed.events {
            let root = changed.log.spans[event.start].start as usize;
            changed.log.bytes[root + native::DISPLAY_NAME] = b'A' + revision % 26;
            changed.log.bytes[root + native::DISPLAY_NAME + 16..root + native::DISPLAY_NAME + 24]
                .copy_from_slice(&1_u64.to_le_bytes());
        }
        let mut cache = Cache::with_retained(&mut retained);
        for event in &changed.events {
            let memory =
                crate::raw::Recorded::build_range(&changed.log, event.start..event.end).unwrap();
            let expected = snapshot(&memory, 0, 7, event.pointer, false).unwrap();
            let actual = cache.decode(&changed, event, 0, 7, &mut shared).unwrap();
            assert_eq!(actual.value, expected);
        }
        cache.finish(&changed);
        drop(cache);
        assert_eq!(retained.state_count(), 4);
    }
}

#[test]
fn retained_witnesses_keep_overlap_failed_read_identity_and_removal_guards() {
    let batch = changing_source_shapes(4);
    let mut retained = Retained::default();
    let mut cache = Cache::with_retained(&mut retained);
    let mut shared = Shared::default();
    let first = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    cache.finish(&batch);
    drop(cache);
    for fault in 0..4 {
        let mut current = changing_source_shapes(1);
        let mut event = current.events[0];
        if fault == 0 {
            current.log.spans[event.start].ok = false;
        }
        if fault == 1 {
            let at = current.log.spans[event.start].start as usize + native::DISPLAY_NAME;
            current.log.bytes[at] = b'Z';
            current.log.bytes[at + 16..at + 24].copy_from_slice(&1_u64.to_le_bytes());
        }
        if fault == 3 {
            let bytes = engine_native::memory::SliceMemory {
                base: event.pointer + native::UPGRADES + 24,
                bytes: &[1, 2, 3, 4],
            };
            current
                .log
                .take_bulk(&bytes, bytes.base, bytes.bytes.len(), 1)
                .unwrap();
            event.end = current.log.spans.len();
        }
        let event = if fault == 2 {
            Event {
                pointer: event.pointer + 1,
                ..event
            }
        } else {
            event
        };
        let mut cache = Cache::with_retained(&mut retained);
        let actual = cache.decode(&current, &event, 0, 7 + 744, &mut shared);
        if let Ok(actual) = actual {
            assert!(!Arc::ptr_eq(&actual, &first));
        }
        assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 0);
    }
    let mut cache = Cache::with_retained(&mut retained);
    cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    cache.remove(first.definition.identity.id);
    cache.finish(&batch);
    drop(cache);
    assert_eq!(retained.state_count(), 0);
}

#[test]
fn retained_missions_compare_operands_after_relocation_without_raw_index_allocations() {
    let batch = relocated_mission_shapes(8);
    let mut retained = Retained::default();
    let mut cache = Cache::with_retained(&mut retained);
    let mut shared = Shared::default();
    let first = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    cache.finish(&batch);
    drop(cache);
    let mut cache = Cache::with_retained(&mut retained);
    let actual = cache
        .decode(&batch, &batch.events[4], 0, 7 + 744, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &actual));
    assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 0);
    assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 1);
    cache.finish(&batch);
    drop(cache);
    assert_eq!(retained.state_count(), 1);
    let mut later = Cache::with_retained(&mut retained);
    let next = later
        .decode(&batch, &batch.events[0], 0, 7 + 1_488, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &next));
    assert_eq!(later.counts[CacheOutcome::FullDecode as usize], 0);
}

#[test]
fn retained_cache_allocation_denials_skip_retention_without_calling_the_terminal_hook() {
    thread_local! { static TERMINAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    fn terminal() {
        TERMINAL.with(|calls| calls.set(calls.get() + 1));
    }
    struct Hook;
    impl Drop for Hook {
        fn drop(&mut self) {
            engine_allocation::set_hooks(None, None);
        }
    }
    let batch = changing_source_shapes(4);
    let mut retained = Retained::default();
    let mut shared = Shared::default();
    let mut cache = Cache::with_retained(&mut retained);
    let decoded = cache
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    let _hook = Hook;
    engine_allocation::set_hooks(None, Some(terminal));
    collector_test_support::with_allocations_denied(|| cache.finish(&batch));
    drop(cache);
    assert_eq!(retained.state_count(), 0);
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    let mut cache = Cache::with_retained(&mut retained);
    let repeated = cache
        .decode(&batch, &batch.events[0], 0, 7 + 744, &mut shared)
        .unwrap();
    assert_eq!(decoded.value, repeated.value);
    assert!(!Arc::ptr_eq(&decoded, &repeated));
}

#[test]
fn the_last_retained_hit_wins_over_an_earlier_local_revision_of_the_same_source_count() {
    let original = changing_source_shapes(1);
    let mut retained = Retained::default();
    let mut shared = Shared::default();
    let mut warm = Cache::with_retained(&mut retained);
    let first = warm
        .decode(&original, &original.events[0], 0, 7, &mut shared)
        .unwrap();
    warm.finish(&original);
    drop(warm);
    let mut batch = changing_source_shapes(2);
    let event = batch.events[0];
    replace_copied_bytes(
        &mut batch,
        &event,
        event.pointer + native::DISPLAY_NAME,
        b"Z",
    );
    replace_copied_bytes(
        &mut batch,
        &event,
        event.pointer + native::DISPLAY_NAME + 16,
        &1_u64.to_le_bytes(),
    );
    let event = batch.events[1];
    replace_copied_bytes(
        &mut batch,
        &event,
        event.pointer + native::BONUS_SOURCES + 12,
        &0_u32.to_le_bytes(),
    );
    let mut cache = Cache::with_retained(&mut retained);
    let changed = cache
        .decode(&batch, &batch.events[0], 0, 751, &mut shared)
        .unwrap();
    assert!(!Arc::ptr_eq(&changed, &first));
    let restored = cache
        .decode(&batch, &batch.events[1], 0, 751, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&restored, &first));
    cache.finish(&batch);
    drop(cache);
    assert_eq!(retained.state_count(), 1);
    let mut later = Cache::with_retained(&mut retained);
    let actual = later
        .decode(&original, &original.events[0], 0, 1_495, &mut shared)
        .unwrap();
    assert!(Arc::ptr_eq(&actual, &first));
}

#[test]
fn removing_fifteen_hundred_designs_visits_only_their_own_source_candidates() {
    let mut cache = Cache::default();
    for id in 1..=1_500 {
        let mut definition = native::DefinitionRow::default();
        definition.identity.id = id;
        definition.design = Arc::new(rows::VariantDesignState::default());
        let value = Arc::new(Decoded {
            value: Default::default(),
            definition,
            context: None,
        });
        assert!(Cache::verified(
            &mut cache.source_heads,
            id as usize,
            0..0,
            &value,
            None,
        ));
    }
    for id in 1..=1_500 {
        let before = cache.removal_visits;
        cache.remove(id);
        assert_eq!(cache.removal_visits - before, 1);
        assert_eq!(cache.source_heads.len(), 1_500 - id as usize);
        cache.remove(id);
        assert_eq!(cache.removal_visits - before, 1);
    }
    assert_eq!(cache.removal_visits, 1_500);
}

enum Lifecycle {
    Epoch,
    Base,
    Census,
    Release,
    Identity,
    Removal,
}
fn retained_lifecycle(change: Lifecycle) {
    for different in [false, true] {
        let original = changing_source_shapes(1);
        let mut current = changing_source_shapes(1);
        let mut retained = Retained::default();
        retained.reset(1, 0);
        let mut shared = Shared::default();
        let mut cache = Cache::with_retained(&mut retained);
        let first = cache
            .decode(&original, &original.events[0], 0, 7, &mut shared)
            .unwrap();
        cache.finish(&original);
        drop(cache);
        assert_eq!(retained.state_count(), 1);
        if different {
            let event = current.events[0];
            replace_copied_bytes(
                &mut current,
                &event,
                event.pointer + native::DISPLAY_NAME,
                b"Z",
            );
            replace_copied_bytes(
                &mut current,
                &event,
                event.pointer + native::DISPLAY_NAME + 16,
                &1_u64.to_le_bytes(),
            );
        }
        match change {
            Lifecycle::Epoch => retained.reset(2, 0),
            Lifecycle::Base => retained.reset(1, 1),
            Lifecycle::Census => {
                retained.prune(&HashMap::new());
                let event = current.events[0];
                let watches = HashMap::from([(
                    event.pointer,
                    Watch {
                        id: event.variant,
                        pointer: event.pointer,
                        flags: [0; 3],
                        dirty: false,
                    },
                )]);
                retained.prune(&watches);
            }
            Lifecycle::Release => {
                retained.release();
                retained.reset(1, 0);
            }
            Lifecycle::Identity => {
                let event = current.events[0];
                let id = event.variant.wrapping_add(1 << 32);
                replace_copied_bytes(
                    &mut current,
                    &event,
                    event.pointer + native::ID,
                    &((id >> 32) as u32).to_le_bytes(),
                );
                current.events[0].variant = id;
                let watches = HashMap::from([(
                    event.pointer,
                    Watch {
                        id,
                        pointer: event.pointer,
                        flags: [0; 3],
                        dirty: false,
                    },
                )]);
                retained.prune(&watches);
            }
            Lifecycle::Removal => {
                let mut cache = Cache::with_retained(&mut retained);
                cache
                    .decode(&original, &original.events[0], 0, 7, &mut shared)
                    .unwrap();
                cache.remove(first.definition.identity.id);
                cache.finish(&original);
            }
        }
        assert_eq!(retained.state_count(), 0);
        let event = &current.events[0];
        let memory =
            crate::raw::Recorded::build_range(&current.log, event.start..event.end).unwrap();
        let expected = snapshot(&memory, 0, 751, event.pointer, false).unwrap();
        let mut cache = Cache::with_retained(&mut retained);
        let returned = cache.decode(&current, event, 0, 751, &mut shared).unwrap();
        assert!(!Arc::ptr_eq(&first, &returned));
        assert_eq!(returned.value, expected);
        assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 1);
        cache.finish(&current);
        drop(cache);
        let mut cache = Cache::with_retained(&mut retained);
        let verified = cache
            .decode(&current, event, 0, 1_495, &mut shared)
            .unwrap();
        assert!(Arc::ptr_eq(&returned, &verified));
        assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 0);
    }
}
#[test]
fn retained_witnesses_do_not_cross_an_epoch_or_base_reset() {
    retained_lifecycle(Lifecycle::Epoch);
    retained_lifecycle(Lifecycle::Base);
}
#[test]
fn retained_witnesses_do_not_cross_census_departure_and_return() {
    retained_lifecycle(Lifecycle::Census);
}
#[test]
fn retained_witnesses_do_not_cross_release_and_reattachment() {
    retained_lifecycle(Lifecycle::Release);
}
#[test]
fn retained_witnesses_do_not_cross_a_new_identity_at_the_same_address() {
    retained_lifecycle(Lifecycle::Identity);
}
#[test]
fn retained_witnesses_do_not_cross_removal_and_readdition_under_the_same_id() {
    retained_lifecycle(Lifecycle::Removal);
}

#[test]
fn the_last_verified_copy_bypasses_indexes_after_local_and_retained_witness_hits() {
    let batch = relocated_mission_shapes(8);
    let mut retained = Retained::default();
    let mut shared = Shared::default();
    let mut warm = Cache::with_retained(&mut retained);
    let first = warm
        .decode(&batch, &batch.events[0], 0, 7, &mut shared)
        .unwrap();
    warm.finish(&batch);
    drop(warm);
    for retention in [false, true] {
        let mut cache = if retention {
            Cache::with_retained(&mut retained)
        } else {
            Cache::default()
        };
        let model = cache
            .decode(&batch, &batch.events[0], 0, 751, &mut shared)
            .unwrap();
        let relocated = cache
            .decode(&batch, &batch.events[4], 0, 751, &mut shared)
            .unwrap();
        assert!(Arc::ptr_eq(&model, &relocated));
        if retention {
            assert!(Arc::ptr_eq(&first, &relocated));
        }
        cache.heads.clear();
        cache.recent.clear();
        cache.witness_heads.clear();
        cache.shape_heads.clear();
        cache.mission_heads.clear();
        cache.mission_models.clear();
        let before = cache.counts;
        let probes = cache.probes;
        for _ in 0..16 {
            let actual = collector_test_support::with_allocations_denied(|| {
                cache
                    .decode(&batch, &batch.events[4], 0, 751, &mut shared)
                    .unwrap()
            });
            assert!(Arc::ptr_eq(&actual, &relocated));
        }
        assert_eq!(
            cache.counts[CacheOutcome::FullCopyHit as usize]
                - before[CacheOutcome::FullCopyHit as usize],
            16
        );
        assert_eq!(
            cache.counts[CacheOutcome::FullDecode as usize],
            before[CacheOutcome::FullDecode as usize]
        );
        assert_eq!(cache.probes, probes);
    }
}

#[test]
fn a_last_verified_copy_still_checks_changed_reads_failed_spans_and_unread_overlaps() {
    for fault in 0..3 {
        let mut batch = changing_source_shapes(1);
        let mut shared = Shared::default();
        let mut cache = Cache::default();
        let event = batch.events[0];
        let first = cache.decode(&batch, &event, 0, 7, &mut shared).unwrap();
        if fault == 0 {
            replace_copied_bytes(
                &mut batch,
                &event,
                event.pointer + native::DISPLAY_NAME,
                b"Z",
            );
            replace_copied_bytes(
                &mut batch,
                &event,
                event.pointer + native::DISPLAY_NAME + 16,
                &1_u64.to_le_bytes(),
            );
        } else if fault == 1 {
            batch.log.spans[event.start].ok = false;
        } else {
            let memory = engine_native::memory::SliceMemory {
                base: event.pointer + native::UPGRADES + 24,
                bytes: &[1, 2, 3, 4],
            };
            batch
                .log
                .take_bulk(&memory, memory.base, memory.bytes.len(), 1)
                .unwrap();
            batch.events[0].end = batch.log.spans.len();
        }
        // Keep the model's immutable bytes separate from the faulted current occurrence.
        let model = changing_source_shapes(1);
        let mut joined = Batch::empty();
        joined.prepare(2, 320, ROOT_BYTES * 12);
        let (start, end) = storage::append(&mut joined.log, &model.log).unwrap();
        joined.events.push(Event {
            start,
            end,
            ..event
        });
        let (start, end) = storage::append(&mut joined.log, &batch.log).unwrap();
        joined.events.push(Event {
            start,
            end,
            sequence: 2,
            ..batch.events[0]
        });
        let mut cache = Cache::default();
        let model = cache
            .decode(&joined, &joined.events[0], 0, 7, &mut shared)
            .unwrap();
        let result = cache.decode(&joined, &joined.events[1], 0, 7, &mut shared);
        if let Ok(actual) = result {
            assert!(!Arc::ptr_eq(&model, &actual));
            if fault == 0 {
                assert_ne!(first.value, actual.value);
            }
        }
        assert_eq!(cache.counts[CacheOutcome::FullCopyHit as usize], 0);
        assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 0);
    }
}

#[test]
fn contiguous_copy_comparison_preserves_sparse_failed_and_metadata_semantics() {
    for case in 0..8 {
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &[1, 2, 3, 4, 5, 6, 7, 8],
        };
        let mut log = RawLog::<1>::new(4, 16);
        for _ in 0..2 {
            log.take_bulk(&memory, memory.base, 4, 1).unwrap();
            log.take_bulk(&memory, memory.base + 4, 4, 1).unwrap();
        }
        match case {
            1 => log.spans[2].start = log.spans[0].start,
            2 => log.bytes[log.spans[2].start as usize] ^= 1,
            3 => log.spans[2].len -= 1,
            4 => {
                log.spans[0].ok = false;
                log.spans[2].ok = false;
            }
            5 => log.spans[2].bulk = false,
            6 => log.spans[2].addr += 1,
            7 => {
                log.spans[0].len = 2;
                log.spans[2].len = 2;
            }
            _ => {}
        }
        let expected = log.spans[..2]
            .iter()
            .zip(&log.spans[2..])
            .all(|(left, right)| {
                left.addr == right.addr
                    && left.len == right.len
                    && left.ok == right.ok
                    && left.bulk == right.bulk
                    && payload(&log, left) == payload(&log, right)
            });
        assert_eq!(equal(&log, 0..2, 2..4), expected, "copy layout case {case}");
    }
}

mod drain_lifecycle;

mod capacity;
