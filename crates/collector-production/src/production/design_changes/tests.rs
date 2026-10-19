use super::super::production_rest_test_kit as kit;
use super::*;

#[test]
fn a_new_fog_refusal_withdraws_the_earlier_daily_snapshot() {
    let mut capture = super::super::Captured::with_capacities(&Capacities::default());
    capture.variants.definitions.outcome = Outcome::Rows { len: 0 };
    capture.variants.fog.snapshot = Some(std::sync::Arc::new(
        rows::TableRows::DesignObserverContext(vec![rows::DesignObserverContextRow { fog: 1 }]),
    ));
    assert!(capture.design_fog().is_some());
    capture.changes.context_acquired = true;
    capture.changes.fog.snapshot = None;
    capture.changes.fog.order = 2;
    assert!(capture.design_fog().is_none());
    capture.variants.fog.order = 3;
    assert!(capture.design_fog().is_some());
}

#[test]
fn shared_design_writers_keep_their_original_receivers_without_owning_patches() {
    assert_eq!(
        FORWARDED_SITES.map(|index| MUTATIONS[index].name),
        [
            "RefreshOrganisation",
            "RefreshOrganisationBonus",
            "MoveAssign"
        ]
    );
    assert!(
        FORWARDED_SITES
            .iter()
            .all(|&index| matches!(MUTATIONS[index].receiver, Receiver::Variant))
    );
    assert_eq!(MUTATIONS.len() - FORWARDED_SITES.len(), 20);
}

fn fixture() -> Vec<u8> {
    let mut bytes = kit::fixture();
    for offset in [
        native::DISPLAY_NAME,
        native::VARIANT_NAME,
        native::EXPLICIT_ENTITY,
        native::ICON_OVERRIDE_TOKEN,
    ] {
        let start = kit::VARIANT - kit::BASE + offset;
        bytes[start..start + 32].fill(0);
        bytes[start + 24..start + 32].copy_from_slice(&15_u64.to_le_bytes());
    }
    let at = kit::VARIANT - kit::BASE + native::UPGRADES;
    bytes[at..at + 8].copy_from_slice(
        &(engine_equipment::cequipment_upgrades_instance::VTABLE_RVA as u64).to_le_bytes(),
    );
    bytes
}
fn edit(bytes: &mut [u8], text: u8, range: i64) {
    let at = kit::VARIANT - kit::BASE;
    bytes[at + native::DISPLAY_NAME] = text;
    bytes[at + native::DISPLAY_NAME + 16..at + native::DISPLAY_NAME + 24]
        .copy_from_slice(&1_u64.to_le_bytes());
    bytes[at + native::MAP_RANGE_STAT..at + native::MAP_RANGE_STAT + 8]
        .copy_from_slice(&range.to_le_bytes());
}
fn append(batch: &mut Batch, bytes: &[u8], sequence: u64, removed: bool) {
    let memory = engine_native::memory::SliceMemory {
        base: kit::BASE,
        bytes,
    };
    let start = batch.log.spans.len();
    collector_test_support::with_allocations_denied(|| {
        stage(&mut batch.log, &memory, 0, kit::VARIANT)
    })
    .unwrap();
    let end = batch.log.spans.len();
    batch.events.push(Event {
        kind: Kind::Design,
        sequence,
        hour: 7,
        pointer: kit::VARIANT,
        variant: 0xfedc_ba98_0000_0046,
        flags: None,
        removed,
        start,
        end,
    });
}

#[test]
fn variant_events_keep_multiple_same_hour_edits_and_final_removal_without_callback_allocation() {
    let mut batch = Batch::empty();
    batch.prepare(4, 256, ROOT_BYTES * 16);
    let mut bytes = fixture();
    edit(&mut bytes, b'A', -100);
    append(&mut batch, &bytes, 1, false);
    edit(&mut bytes, b'B', 200);
    append(&mut batch, &bytes, 2, false);
    append(&mut batch, &bytes, 3, true);
    // Later contents at the same native addresses cannot change owned events.
    edit(&mut bytes, b'C', 300);
    let mut rows = Vec::new();
    decode_batch(&batch, 0, 7, &mut rows).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[..2]
            .iter()
            .map(|r| r
                .snapshot
                .as_ref()
                .unwrap()
                .definition
                .display_name
                .as_slice())
            .collect::<Vec<_>>(),
        [b"A", b"B"]
    );
    assert_eq!(
        rows[..2]
            .iter()
            .map(|r| r.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [-100, 200]
    );
    assert!(!rows[0].removed && !rows[1].removed && rows[2].removed);
    assert!(rows[2].snapshot.is_none());
    assert!(!unchanged(&rows[2], &rows[..2], None));
    let mut duplicate = rows[1].clone();
    duplicate.sequence = 4;
    duplicate.removed = false;
    assert!(unchanged(&duplicate, &rows[..2], None));
    for row in rows {
        rows::RowCodec::check_key_fields(&row).unwrap();
    }
}

#[test]
fn variant_named_sources_keep_country_scope_duplicates_and_owned_values() {
    let mut bytes = fixture();
    let put = |bytes: &mut [u8], address: usize, value: &[u8]| {
        bytes[address - kit::BASE..address - kit::BASE + value.len()].copy_from_slice(value);
    };
    let array = |bytes: &mut [u8], address: usize, data: usize, count: u32| {
        put(bytes, address, &(data as u64).to_le_bytes());
        put(bytes, address + 8, &count.to_le_bytes());
        put(bytes, address + 12, &count.to_le_bytes());
    };
    put(
        &mut bytes,
        kit::VARIANT + native::CREATOR_LINK,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        kit::VARIANT + native::ORIGIN_LINK,
        &2_u32.to_le_bytes(),
    );
    array(&mut bytes, kit::VARIANT + native::NAMED_BONUSES, 0x9000, 4);
    for (index, handle) in [17_u32, 7, u32::MAX, 7].iter().enumerate() {
        put(&mut bytes, 0x9000 + index * 4, &handle.to_le_bytes());
    }
    array(
        &mut bytes,
        0x10000 + native::COUNTRY_NAMED_BONUSES,
        0x9100,
        1,
    );
    put(
        &mut bytes,
        0x9100,
        &(native::NAMED_BONUS_VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x9108,
        &(native::SPECIFIC_BONUS_VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x9100 + native::NAMED_BONUS_HANDLE,
        &7_u32.to_le_bytes(),
    );
    for (offset, text) in [
        (native::NAMED_BONUS_NAME, b"name".as_slice()),
        (native::NAMED_BONUS_DESCRIPTION, b"description".as_slice()),
    ] {
        put(&mut bytes, 0x9100 + offset, text);
        put(
            &mut bytes,
            0x9100 + offset + 16,
            &(text.len() as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x9100 + offset + 24, &15_u64.to_le_bytes());
    }
    array(&mut bytes, 0x9108 + native::BONUS_STATS, 0x9300, 1);
    put(&mut bytes, 0x9300, &5_u32.to_le_bytes());
    put(&mut bytes, 0x9308, &27_000_i64.to_le_bytes());
    struct World<'a>(&'a [u8]);
    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&(kit::BASE as u64).to_le_bytes());
                true
            } else {
                engine_native::memory::SliceMemory {
                    base: kit::BASE,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
    }
    let mut batch = Batch::empty();
    batch.prepare(1, 256, ROOT_BYTES * 20);
    collector_test_support::with_allocations_denied(|| {
        stage(&mut batch.log, &World(&bytes), 0, kit::VARIANT)
    })
    .unwrap();
    batch.events.push(Event {
        kind: Kind::Design,
        sequence: 1,
        hour: 7,
        pointer: kit::VARIANT,
        variant: 0xfedc_ba98_0000_0046,
        flags: None,
        removed: false,
        start: 0,
        end: batch.log.spans.len(),
    });
    put(&mut bytes, 0x9308, &99_000_i64.to_le_bytes());
    let mut rows = Vec::new();
    decode_batch(&batch, 0, 7, &mut rows).unwrap();
    let state = rows[0]
        .snapshot
        .as_ref()
        .unwrap()
        .definition
        .design
        .require("design")
        .unwrap();
    assert_eq!(state.named_bonuses, [17, 7, u32::MAX, 7]);
    assert_eq!(state.named_sources.len(), 2);
    assert_eq!(state.named_sources[0], state.named_sources[1]);
    let dictionary = super::super::bonus_snapshots::snapshot().unwrap().unwrap();
    let source = &dictionary
        .rows::<tables::VariantBonusSnapshots>()
        .unwrap()
        .iter()
        .find(|r| r.id == state.named_sources[0])
        .unwrap()
        .source;
    assert_eq!(
        source.stats,
        [rows::VariantStatEntry {
            index: 5,
            value: 27_000
        }]
    );
    let named = source.named.as_ref().unwrap();
    assert_eq!((named.owner, named.handle), (2, 7));
    assert_eq!(named.name, b"name");
    assert_eq!(named.description, b"description");
}

#[test]
fn variant_raw_capacity_pressure_refuses_instead_of_allocating_or_losing_coverage_silently() {
    let bytes = fixture();
    let memory = engine_native::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let mut batch = Batch::empty();
    assert!(
        collector_test_support::with_allocations_denied(|| stage(
            &mut batch.log,
            &memory,
            0,
            kit::VARIANT
        ))
        .is_err()
    );
    assert!(batch.events.is_empty());
    batch.prepare(1, 64, ROOT_BYTES * 4);
    batch.clear();
    assert!(
        collector_test_support::with_allocations_denied(|| stage(
            &mut batch.log,
            &memory,
            0,
            kit::VARIANT
        ))
        .is_ok()
    );
}

#[test]
fn variant_version_ceiling_observes_only_its_native_ancestry_owner() {
    let mut bytes = vec![0; 0x1400];
    let parent = |bytes: &mut [u8], object: usize, value: usize| {
        let offset = object - 0x1000 + native::PARENT;
        bytes[offset..offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
    };
    let origin = |bytes: &mut [u8], object: usize, value: i32| {
        let offset = object - 0x1000 + native::ORIGIN_LINK;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    parent(&mut bytes, 0x1000, 0x1800);
    parent(&mut bytes, 0x1800, 0x2000);
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        collector_test_support::with_allocations_denied(|| version_owner(&memory, 0x1000, 3))
            .unwrap(),
        0x2000
    );
    origin(&mut bytes, 0x1000, 9);
    origin(&mut bytes, 0x1800, 9);
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(version_owner(&memory, 0x1000, 3).unwrap(), 0x1800);
    origin(&mut bytes, 0x2000, 9);
    parent(&mut bytes, 0x2000, 0x1000);
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(version_owner(&memory, 0x1000, 3).is_err());
}

fn input(hour: i32) -> replay_frame::frame::FrameInput {
    replay_frame::frame::FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
        metrics: replay_frame::frame::Metrics::default(),
    }
}

#[test]
fn cold_and_post_gap_keyframes_publish_fresh_designs_or_named_refusals() {
    use std::sync::Arc;
    let bytes = fixture();
    let memory = engine_native::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let registry = [native::RegistryRow::read(&memory, 0, kit::VARIANT).unwrap()];
    let mut captured = super::super::Captured::with_capacities(&kit::capacities());
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    for hour in [7, 9] {
        captured.variants.reset();
        captured.changes.outcome = Outcome::Unavailable {
            reason: "variant change observer not prepared",
        };
        assert!(matches!(
            captured.design_changes_outcome(),
            Outcome::Unavailable { .. }
        ));
        captured.variants.capture(
            &memory,
            0,
            Sources {
                rows: &registry,
                outcome: Outcome::Rows { len: 1 },
            },
            hour,
            true,
        );
        let mut refused = input(hour);
        for (table, value) in TABLES.into_iter().zip(captured.table_inputs(None)) {
            refused.tables[table.index()] = value;
        }
        // The same ordinary encoder used by the spool rejects missing fields;
        // a cold hook refusal must stay an explicit table verdict instead.
        replay_frame::frame::FrameEncoder::new()
            .encode_input(refused, true)
            .unwrap();

        // Model the worker's successful three-certificate baseline handoff.
        captured.variants.baseline_admitted = true;
        captured.variants.dictionary =
            Some(Arc::new(rows::TableRows::VariantBonusSnapshots(vec![
                rows::VariantBonusSnapshotRow {
                    id: 1,
                    source: Default::default(),
                },
            ])));
        captured.variants.fog.snapshot =
            Some(Arc::new(rows::TableRows::DesignObserverContext(vec![
                rows::DesignObserverContextRow { fog: 1 },
            ])));
        captured.variants.liveness = liveness::Observation {
            order: u64::MAX,
            admitted: true,
            snapshot: Some(Arc::new(rows::TableRows::DesignOrganisationLiveness(vec![
                rows::DesignOrganisationLivenessRow {
                    organisation: 79,
                    live: true,
                },
            ]))),
        };
        let mut fresh = input(hour);
        for (table, value) in TABLES.into_iter().zip(captured.table_inputs(None)) {
            fresh.tables[table.index()] = value;
        }
        let wire = encoder.encode_input(fresh, true).unwrap();
        let frame = decoder.apply(&wire).unwrap();
        for table in [
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantStats,
            TableId::VariantUpgrades,
            TableId::VariantChanges,
            TableId::VariantBonusSnapshots,
            TableId::DesignObserverContext,
            TableId::DesignOrganisationLiveness,
        ] {
            assert!(
                matches!(&*frame.tables[table.index()], TableView::Complete(_)),
                "{hour}: {table:?}"
            );
        }
        assert_eq!(
            frame
                .complete::<tables::VariantDefinitions>()
                .unwrap()
                .len(),
            1
        );
        assert!(
            frame
                .complete::<tables::VariantChanges>()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn liveness_refusal_and_recovery_are_dated_at_acquisition_not_projection() {
    use std::sync::Arc;
    let mut captured = super::super::Captured::with_capacities(&Capacities::default());
    let payload = Arc::new(rows::TableRows::DesignOrganisationLiveness(vec![
        rows::DesignOrganisationLivenessRow {
            organisation: 79,
            live: true,
        },
    ]));
    let first = liveness::Observation::refused();
    captured.variants.liveness = liveness::Observation {
        order: first.order,
        admitted: true,
        snapshot: Some(Arc::clone(&payload)),
    };
    assert!(Arc::ptr_eq(
        captured.design_liveness().as_ref().unwrap(),
        &payload
    ));
    captured.changes.liveness = liveness::Observation::refused();
    assert!(captured.design_liveness().is_none());
    captured.variants.liveness = liveness::Observation {
        order: liveness::Observation::refused().order,
        admitted: true,
        snapshot: Some(Arc::clone(&payload)),
    };
    assert!(Arc::ptr_eq(
        captured.design_liveness().as_ref().unwrap(),
        &payload
    ));
    captured
        .variants
        .fail(replay_data::cadence::SKIPPED_REASON, 7, false);
    assert!(
        Arc::ptr_eq(captured.design_liveness().as_ref().unwrap(), &payload),
        "a skipped census must keep its observation date"
    );
    captured.changes.liveness = liveness::Observation {
        order: liveness::Observation::refused().order,
        snapshot: Some(Arc::clone(&payload)),
        admitted: true,
    };
    let references = Arc::strong_count(&payload);
    collector_test_support::with_allocations_denied(|| captured.changes.fail("lost observer job"));
    assert!(captured.design_liveness().is_none());
    assert_eq!(Arc::strong_count(&payload), references);
}

#[test]
fn variant_current_census_compares_deep_state_without_allocating_and_clones_only_changed_rows() {
    let state = std::sync::Arc::new(rows::VariantDesignState {
        niches: vec![1, 2, 3],
        terrain_attrition: vec![9; 32],
        ..Default::default()
    });
    let mut first = input(21);
    first.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(rows::TableRows::VariantDefinitions(
            (1..=5_000)
                .map(|id| rows::VariantDefinitionRow {
                    id,
                    version: rows::Recorded::Value(0),
                    design: rows::Recorded::Value(state.as_ref().clone()),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let first_wire = encoder.encode_input(first, false).unwrap();
    let scratch = crate::projection::ProjectionScratch::default();
    let projection = Projection::new(&encoder, 1, 22, false, &scratch);
    let mut census = Variants::vacant();
    census.definitions.rows = (1..=5_000)
        .map(|id| native::DefinitionRow {
            identity: native::Row {
                id,
                ..Default::default()
            },
            design: std::sync::Arc::clone(&state),
            ..Default::default()
        })
        .collect();
    census.definitions.outcome = Outcome::Rows { len: 5_000 };
    let quiet = collector_test_support::with_allocations_denied(|| {
        super::super::definitions_input(
            Some(&projection),
            &census.definitions,
            &census.definition_bytes,
        )
    });
    assert!(matches!(quiet, TableInput::ComparedEqual(_)));
    census.definitions.rows.reverse();
    collector_test_support::with_allocations_denied(|| census.canonicalize_definitions());
    let reordered = collector_test_support::with_allocations_denied(|| {
        super::super::definitions_input(
            Some(&projection),
            &census.definitions,
            &census.definition_bytes,
        )
    });
    assert!(matches!(reordered, TableInput::ComparedEqual(_)));
    let mut changed = state.as_ref().clone();
    changed.manpower = 9;
    census.definitions.rows[36].design = std::sync::Arc::new(changed);
    let edited = super::super::definitions_input(
        Some(&projection),
        &census.definitions,
        &census.definition_bytes,
    );
    let TableInput::ComparedChanges { indices, rows, .. } = &edited else {
        panic!("expected one current census change");
    };
    assert_eq!(indices, &[36]);
    let values = rows.rows::<tables::VariantDefinitions>().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].id, 37);
    assert_eq!(values[0].design.require("design").unwrap().manpower, 9);
    census.definitions.rows[36].identity.id = 36;
    assert!(matches!(
        super::super::definitions_input(Some(&projection), &census.definitions, &census.definition_bytes),
        TableInput::Sampled(TableView::Unavailable { reason }) if reason.contains("duplicate engine ids")
    ));
    let mut second = input(22);
    second.tables[TableId::VariantDefinitions.index()] = edited;
    let second_wire = encoder.encode_input(second, false).unwrap();
    let mut third = input(23);
    third.tables[TableId::VariantDefinitions.index()] =
        unavailable(replay_data::cadence::SKIPPED_REASON).into();
    let third_wire = encoder.encode_input(third, false).unwrap();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    decoder.apply(&first_wire).unwrap();
    decoder.apply(&second_wire).unwrap();
    let carried = decoder.apply(&third_wire).unwrap();
    assert_eq!(
        carried.complete::<tables::VariantDefinitions>().unwrap()[36]
            .design
            .require("design")
            .unwrap()
            .manpower,
        9
    );
}

#[test]
fn variant_observed_edits_replace_only_the_changed_design_and_keep_last_edit() {
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let scratch = crate::projection::ProjectionScratch::default();
    let definitions: Vec<_> = (1..=5_000)
        .map(|id| rows::VariantDefinitionRow {
            id,
            design: rows::Recorded::Value(rows::VariantDesignState::default()),
            ..Default::default()
        })
        .collect();
    let mut first = input(0);
    first.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(rows::TableRows::VariantDefinitions(definitions)).into();
    encoder.encode_input(first, false).unwrap();
    let snapshot = |name: &[u8]| rows::VariantChangeRow {
        sequence: 1,
        hour: 1,
        variant: 37,
        snapshot: Some(rows::VariantSnapshot {
            definition: rows::VariantDefinitionRow {
                id: 37,
                display_name: name.to_vec(),
                design: rows::Recorded::Value(rows::VariantDesignState::default()),
                ..Default::default()
            }
            .into(),
            stats: vec![0; 10],
            ..Default::default()
        }),
        ..Default::default()
    };
    let history = [
        snapshot(b"first"),
        rows::VariantChangeRow {
            sequence: 2,
            hour: 1,
            variant: 37,
            scalar: Some(rows::VariantScalarChange {
                display_name: Some(b"last".to_vec()),
                auto_upgraded: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        },
        rows::VariantChangeRow {
            sequence: 3,
            hour: 1,
            variant: 37,
            flags: Some([1, 0, 1]),
            ..Default::default()
        },
    ];
    let projection = Projection::new(&encoder, 1, 1, false, &scratch);
    let projected = project_definitions(&history, Some(&projection));
    let TableInput::ComparedChanges { indices, rows, .. } = &projected else {
        panic!("expected sparse design replacement");
    };
    assert_eq!(indices, &[36]);
    let row = &rows.rows::<tables::VariantDefinitions>().unwrap()[0];
    assert_eq!(row.display_name, b"last");
    assert_eq!(row.design.require("design").unwrap().auto_upgraded, 1);
    assert_eq!(row.design.require("design").unwrap().can_upgrade_modules, 1);
    let mut next = input(1);
    next.tables[TableId::VariantDefinitions.index()] = projected;
    encoder.encode_input(next, false).unwrap();
    let (_, rows) = encoder.committed::<tables::VariantDefinitions>().unwrap();
    assert_eq!(rows.len(), 5_000);
    assert_eq!(rows[36].display_name, b"last");
    assert!(rows[35].display_name.is_empty());
    let flags = Captured {
        rows: vec![history[2].clone()],
        outcome: Outcome::Rows { len: 1 },
        heap_bytes: 0,
        dictionary: None,
        context: None,
        templates: None,
        fog: super::super::design_fog::Observation::default(),
        liveness: liveness::Observation::default(),
        context_acquired: false,
    };
    let projection = Projection::new(&encoder, 1, 2, false, &scratch);
    let [definitions, modules, stats, upgrades] = project(&flags, Some(&projection));
    assert!(matches!(definitions, TableInput::ComparedEqual(_)));
    for table in [modules, stats, upgrades] {
        assert!(
            matches!(table, TableInput::Sampled(TableView::Unavailable { reason }) if reason == replay_data::cadence::SKIPPED_REASON)
        );
    }
}

#[test]
fn variant_frame_reset_withdraws_old_observations_and_keeps_reusable_storage() {
    let mut captured = Captured {
        rows: vec![rows::VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: 37,
            flags: Some([1, 0, 1]),
            ..Default::default()
        }],
        outcome: Outcome::Rows { len: 1 },
        heap_bytes: 0,
        dictionary: Some(std::sync::Arc::new(rows::TableRows::VariantBonusSnapshots(
            Vec::new(),
        ))),
        context_acquired: true,
        templates: None,
        fog: super::super::design_fog::Observation::default(),
        liveness: liveness::Observation::default(),
        context: Some(std::sync::Arc::new(
            rows::TableRows::DesignCountryModifiers(Vec::new()),
        )),
    };
    let capacity = captured.rows.capacity();
    captured.outcome = Outcome::Unavailable {
        reason: "design audit refused",
    };
    assert!(
        captured.context().is_some(),
        "independent country certificate survives design refusal"
    );
    let pointer = captured.rows.as_ptr();
    let dictionary = std::sync::Arc::clone(captured.dictionary.as_ref().unwrap());
    let references = std::sync::Arc::strong_count(&dictionary);
    collector_test_support::with_allocations_denied(|| captured.reset());
    assert_eq!(captured.rows.len(), 1);
    assert_eq!(captured.rows.as_ptr(), pointer);
    assert_eq!(captured.rows.capacity(), capacity);
    assert_eq!(std::sync::Arc::strong_count(&dictionary), references);
    assert!(captured.context.is_some());
    assert!(captured.dictionary().is_none());
    assert!(captured.context().is_none());
    collector_test_support::with_allocations_denied(|| captured.fail("fixture lost worker loan"));
    assert_eq!(captured.rows.len(), 1);
    assert_eq!(captured.rows.as_ptr(), pointer);
    assert_eq!(std::sync::Arc::strong_count(&dictionary), references);
    assert!(captured.context.is_some());
    assert!(captured.dictionary().is_none());
    assert!(captured.context().is_none());
    for source in [&captured, &Captured::vacant()] {
        assert!(matches!(source.outcome, Outcome::Unavailable { .. }));
        assert!(matches!(
            source.table_input(None),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }
}

#[test]
fn variant_site_authentication_without_dependency_baselines_never_admits_carry() {
    assert!(native::mutations::WRITER_AUDIT_REFUSAL.is_none());
    assert!(held::certified().is_none());
    let memory = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let hooks = unsafe { Hooks::new(&memory, 0) }.unwrap();
    assert!(hooks.hooks.is_empty());
}

#[test]
fn base_destruction_uses_only_the_prepared_binding_and_clock_without_shifting_or_allocation() {
    struct Clock;
    impl Memory for Clock {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            if address == engine_native::cgame_state::SINGLETON_RVA && bytes.len() == 8 {
                bytes.copy_from_slice(&0x1000_u64.to_le_bytes());
                true
            } else if address == 0x1000 + engine_native::cgame_state::CURRENT_HOUR
                && bytes.len() == 4
            {
                bytes.copy_from_slice(&7_i32.to_le_bytes());
                true
            } else {
                panic!("destruction must not read the variant or its payload");
            }
        }
    }
    let mut store = Store {
        batch: Batch::empty(),
        spare: Batch::empty(),
        watched: HashMap::with_capacity(10_000),
        interests: sources::Interests::default(),
        epoch: EPOCH.load(Ordering::Acquire),
    };
    store.batch.prepare(1, 0, 0);
    for index in 1..=5_000 {
        store.watched.insert(
            index,
            Watch {
                id: index as u64,
                pointer: index,
                flags: [0; 3],
            },
        );
    }
    let capacity = store.watched.capacity();
    collector_test_support::with_allocations_denied(|| {
        retire_from(&mut store, &Clock, 0, 2_500);
        retire_from(&mut store, &Clock, 0, 2_500);
        store.watched.insert(
            2_500,
            Watch {
                id: 9_999,
                pointer: 2_500,
                flags: [0; 3],
            },
        );
    });
    assert_eq!(store.watched.capacity(), capacity);
    assert_eq!(store.watched[&2_500].id, 9_999);
    assert_eq!(store.watched[&2_499].id, 2_499);
    assert_eq!(store.batch.events.len(), 1);
    let event = &store.batch.events[0];
    assert!(event.removed);
    assert_eq!((event.variant, event.start, event.end), (2_500, 0, 0));
    let mut output = Vec::new();
    decode_batch(&store.batch, 0, 7, &mut output).unwrap();
    assert!(output[0].removed && output[0].snapshot.is_none());
    rows::RowCodec::check_key_fields(&output[0]).unwrap();
}
