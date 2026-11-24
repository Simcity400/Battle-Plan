use super::super::production_rest_test_kit as kit;
use super::*;

pub(super) fn with_store_contended<R: Send>(callback: impl FnOnce() -> R + Send) -> R {
    static WAIT: OnceLock<std::sync::Barrier> = OnceLock::new();
    fn waiting() {
        WAIT.get().unwrap().wait();
    }
    let pause = WAIT.get_or_init(|| std::sync::Barrier::new(2));
    let store = STORE.get().unwrap().lock().unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| {
            BEFORE_STORE_WAIT.set(Some(waiting));
            collector_test_support::with_allocations_denied(callback)
        });
        pause.wait();
        drop(store);
        writer.join().unwrap()
    })
}

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

pub(super) fn fixture() -> Vec<u8> {
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
        &(engine_equipment::cequipment_upgrades_instance::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    bytes
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
        &(native::NAMED_BONUS_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x9108,
        &(native::SPECIFIC_BONUS_VTABLE_RVA.0 as u64).to_le_bytes(),
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
            if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&(kit::BASE as u64).to_le_bytes());
                true
            } else {
                engine_native_memory::memory::SliceMemory {
                    base: kit::BASE,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
    }
    let value = snapshot(&World(&bytes), 0, 7, kit::VARIANT, false).unwrap();
    put(&mut bytes, 0x9308, &99_000_i64.to_le_bytes());
    let state = value.definition.design.require("design").unwrap();
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
    let memory = engine_native_memory::memory::SliceMemory {
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
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(version_owner(&memory, 0x1000, 3).unwrap(), 0x1800);
    origin(&mut bytes, 0x2000, 9);
    parent(&mut bytes, 0x2000, 0x1000);
    let memory = engine_native_memory::memory::SliceMemory {
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
    let memory = engine_native_memory::memory::SliceMemory {
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
        replay_frame::frame::FrameEncoder::new()
            .encode_input(refused, true)
            .unwrap();

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
            stats: vec![Some(0); 10],
            ..Default::default()
        }),
        ..Default::default()
    };
    let equal = [snapshot(b"")];
    let projection = Projection::new(&encoder, 1, 1, false, &scratch);
    assert!(matches!(
        collector_test_support::with_allocations_denied(|| {
            project_definitions(snapshots::History::from_rows(&equal), Some(&projection))
        }),
        TableInput::ComparedEqual(_)
    ));
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
    let projected = project_definitions(snapshots::History::from_rows(&history), Some(&projection));
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
        snapshots: snapshots::Shared::default(),
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
        snapshots: snapshots::Shared::default(),
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
    captured.release_transient();
    assert_eq!(captured.rows.capacity(), 0);
    assert!(captured.dictionary.is_none());
    assert!(captured.context.is_none());
    assert!(captured.templates.is_none());
}

#[test]
fn variant_site_authentication_without_dependency_baselines_never_admits_carry() {
    assert!(native::mutations::WRITER_AUDIT_REFUSAL.is_none());
    assert!(held::certified().is_none());
    let memory = engine_native_memory::memory::SliceMemory {
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
            if address == engine_native::cgame_state::SINGLETON_RVA.0 && bytes.len() == 8 {
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
        published: Published::default(),
        interests: sources::Interests::default(),
        epoch: EPOCH.load(Ordering::Acquire),
    };
    store.batch.prepare(1);
    for index in 1..=5_000 {
        store.watched.insert(
            index,
            Watch {
                id: index as u64,
                pointer: index,
                flags: [0; 3],
                dirty: false,
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
                dirty: false,
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

#[test]
fn variant_rejected_writer_rechecks_saved_membership_after_an_admitted_move() {
    if !isolated() {
        return;
    }
    use std::sync::Barrier;

    unsafe extern "system-unwind" fn move_design(
        destination: usize,
        source: usize,
        pause: usize,
        _: usize,
    ) -> usize {
        let pause = unsafe { &*(pause as *const Barrier) };
        pause.wait();
        pause.wait();
        unsafe {
            std::ptr::copy_nonoverlapping(source as *const u8, destination as *mut u8, ROOT_BYTES);
        }
        19
    }
    unsafe extern "system-unwind" fn highlight(
        pointer: usize,
        pause: usize,
        _: usize,
        _: usize,
    ) -> usize {
        let pause = unsafe { &*(pause as *const Barrier) };
        pause.wait();
        pause.wait();
        unsafe { std::ptr::write((pointer + native::HIGHLIGHT) as *mut u8, 1) };
        23
    }

    let space = super::super::variant_scale_tests::space(1, 2);
    let mut registry = Vec::new();
    assert_eq!(
        native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry),
        Outcome::Rows { len: 2 }
    );
    let mut variants = Variants::vacant();
    variants.capture_state_carried(
        &space.memory(),
        space.base,
        Sources {
            rows: &registry,
            outcome: Outcome::Rows { len: 2 },
        },
        7,
        true,
        false,
    );
    assert_eq!(variants.definitions.outcome, Outcome::Rows { len: 2 });
    BASE.store(space.base, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
    collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
    begin_read();
    prepare(&registry, &variants);
    assert!(baseline_admitted());

    let destination = registry[0].pointer();
    let source = registry[1].pointer();
    let move_pause = Barrier::new(2);
    let move_args = [
        destination,
        source,
        &move_pause as *const Barrier as usize,
        0,
    ];
    let moving = std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe {
            around::<9>(move_args, move_design)
        })
    });
    move_pause.wait();
    let retired = !STORE
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .watched
        .contains_key(&destination);

    let scalar_pause = Barrier::new(2);
    let scalar_args = [destination, &scalar_pause as *const Barrier as usize, 0, 0];
    let scalar = std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe {
            around::<6>(scalar_args, highlight)
        })
    });
    scalar_pause.wait();
    let entry_missed = MISSED.load(Ordering::Acquire);
    let overlapping = active();
    move_pause.wait();
    let move_result = moving.join().unwrap();
    let rebound = STORE
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .watched
        .contains_key(&destination);
    let before_store_missed = MISSED.swap(false, Ordering::AcqRel);
    repair::take(&mut Batch::empty()).unwrap();
    STORE
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .watched
        .get_mut(&destination)
        .unwrap()
        .dirty = false;
    scalar_pause.wait();
    let scalar_result = scalar.join().unwrap();

    assert!(retired && rebound);
    assert_eq!(overlapping, 2);
    assert!(!entry_missed && before_store_missed);
    assert_eq!((move_result, scalar_result), (19, 23));
    assert_eq!(active(), 0);
    {
        let store = STORE.get().unwrap().lock().unwrap();
        assert_eq!(store.batch.failure, None);
        assert_eq!(store.batch.events.len(), 2);
        assert!(store.batch.events.iter().all(|event| event.removed));
        assert!(repair::pending());
        assert_eq!(
            engine_native_memory::read::<u8>(&space.memory(), destination + native::HIGHLIGHT),
            Some(1)
        );
    }
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 3);
    assert!(changes[0].removed && changes[1].removed);
    assert_eq!(
        [changes[0].variant, changes[1].variant],
        [registry[0].id, registry[1].id]
    );
    assert_eq!(changes[2].variant, registry[1].id);
    let snapshot = changes[2].snapshot.as_ref().unwrap();
    assert_eq!(snapshot.definition.design.require("design").unwrap().highlight, 1);
    assert_eq!(MISSED_REFUSALS.load(Ordering::Relaxed), 0);
    assert_eq!(RUNNING_REFUSALS.load(Ordering::Relaxed), 0);
}

mod concurrency;

#[test]
fn variant_membership_projection_keeps_terminal_rows_and_checks_every_occurrence() {
    let baseline: Vec<_> = (1..=5_000)
        .map(|variant| rows::VariantUpgradeRow {
            variant,
            source_ordinal: 0,
            definition_token: 17,
            level: 1,
        })
        .collect();
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut first = input(0);
    first.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(rows::TableRows::VariantUpgrades(baseline.clone())).into();
    encoder.encode_input(first, false).unwrap();
    let event = |variant, levels: &[u8]| rows::VariantChangeRow {
        variant,
        snapshot: Some(rows::VariantSnapshot {
            definition: rows::VariantDefinitionRow {
                id: variant,
                ..Default::default()
            }
            .into(),
            upgrades: levels
                .iter()
                .enumerate()
                .map(|(index, &level)| rows::VariantSnapshotUpgrade {
                    ordinal: index as u32,
                    definition_token: 17,
                    level,
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let history = [
        event(37, &[2, 3]),
        rows::VariantChangeRow {
            variant: 3,
            removed: true,
            ..Default::default()
        },
        event(37, &[4]),
        event(5_001, &[5, 6]),
        event(2, &[]),
        event(3, &[7]),
    ];
    let make = |snapshot: &rows::VariantSnapshot| {
        engine_allocation::collect_vec(snapshot.upgrades.iter().map(|row| {
            rows::VariantUpgradeRow {
                variant: snapshot.definition.id,
                source_ordinal: row.ordinal,
                definition_token: row.definition_token,
                level: row.level,
            }
        }))
    };
    let mut expected = baseline;
    for event in &history {
        expected.retain(|row| row.variant != event.variant);
        if !event.removed {
            expected.extend(make(event.snapshot.as_ref().unwrap()).unwrap());
        }
    }
    expected.sort_unstable_by_key(rows::RowCodec::key);
    let scratch = crate::projection::ProjectionScratch::default();
    for force in [false, true] {
        let projection = Projection::new(&encoder, 1, 1, force, &scratch);
        let result = apply::<tables::VariantUpgrades>(
            snapshots::History::from_rows(&history),
            Some(&projection),
            |row| row.variant,
            make,
        );
        assert!(
            matches!(result, TableInput::Sampled(TableView::Complete(ref rows))
            if rows.rows::<tables::VariantUpgrades>() == Some(expected.as_slice()))
        );
    }
    let projection = Projection::new(&encoder, 1, 1, true, &scratch);
    let bad = [
        rows::VariantChangeRow {
            variant: 37,
            ..Default::default()
        },
        rows::VariantChangeRow {
            variant: 37,
            removed: true,
            ..Default::default()
        },
    ];
    assert!(
        matches!(apply::<tables::VariantUpgrades>(snapshots::History::from_rows(&bad), Some(&projection), |row| row.variant, make),
        TableInput::Sampled(TableView::Unavailable { reason })
        if reason == "variant changes have an incomplete observation")
    );
}

#[test]
fn reused_definition_edits_keep_intermediate_validation_refusals() {
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut first = input(0);
    first.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(rows::TableRows::VariantDefinitions(vec![
            rows::VariantDefinitionRow {
                id: 37,
                design: rows::Recorded::Value(rows::VariantDesignState::default()),
                ..Default::default()
            },
        ]))
        .into();
    encoder.encode_input(first, false).unwrap();
    let source = |direct| rows::VariantChangeRow {
        variant: 37,
        sources: Some(rows::VariantSourceReferences {
            direct,
            named: Vec::new(),
        }),
        ..Default::default()
    };
    let scratch = crate::projection::ProjectionScratch::default();
    let projection = Projection::new(&encoder, 1, 1, false, &scratch);
    for next in [
        rows::VariantChangeRow {
            variant: 37,
            scalar: Some(Default::default()),
            ..Default::default()
        },
        source(vec![1]),
    ] {
        let scalar = next.scalar.is_some();
        let result = project_definitions(
            snapshots::History::from_rows(&[source(vec![0]), next, source(vec![1])]),
            Some(&projection),
        );
        let expected = if scalar {
            "variant scalar projection allocation failed"
        } else {
            "variant source change allocation failed"
        };
        assert!(
            matches!(result, TableInput::Sampled(TableView::Unavailable { reason })
            if reason == expected)
        );
    }
}

#[test]
fn shared_variant_projection_reuses_quiet_groups_and_keeps_identical_frame_bytes() {
    use std::sync::Arc;

    let definitions: Vec<_> = (1..=5_000)
        .map(|id| rows::VariantDefinitionRow {
            id,
            design: rows::Recorded::Value(rows::VariantDesignState::default()),
            ..Default::default()
        })
        .collect();
    let snapshot = rows::VariantSnapshot {
        definition: definitions[36].clone().into(),
        modules: vec![rows::VariantSnapshotModule {
            ordinal: 0,
            slot: 11,
            definition_token: 17,
            active: 1,
        }],
        stats: rows::VariantStatsRow::default().values.to_vec(),
        upgrades: vec![rows::VariantSnapshotUpgrade {
            ordinal: 0,
            definition_token: 19,
            level: 2,
        }],
    };
    let baseline = [
        Arc::new(rows::TableRows::VariantDefinitions(definitions)),
        Arc::new(rows::TableRows::VariantModules(
            (1..=5_000)
                .map(|variant| rows::VariantModuleRow {
                    variant,
                    source_ordinal: 0,
                    slot: 11,
                    definition_token: 17,
                    definition_active: 1,
                })
                .collect(),
        )),
        Arc::new(rows::TableRows::VariantStats(
            (1..=5_000)
                .map(|variant| rows::VariantStatsRow {
                    variant,
                    ..Default::default()
                })
                .collect(),
        )),
        Arc::new(rows::TableRows::VariantUpgrades(
            (1..=5_000)
                .map(|variant| rows::VariantUpgradeRow {
                    variant,
                    source_ordinal: 0,
                    definition_token: 19,
                    level: 2,
                })
                .collect(),
        )),
    ];
    for force in [false, true] {
        let mut optimized = replay_frame::frame::FrameEncoder::new();
        let mut reference = replay_frame::frame::FrameEncoder::new();
        for encoder in [&mut optimized, &mut reference] {
            let mut first = input(0);
            for rows in &baseline {
                first.tables[rows.table().index()] = TableInput::SharedComplete(Arc::clone(rows));
            }
            encoder.encode_input(first, false).unwrap();
        }
        let scratch = crate::projection::ProjectionScratch::default();
        for hour in 1..=4 {
            let mut snapshot = snapshot.clone();
            if hour >= 2 {
                snapshot.definition.display_name = b"changed".to_vec();
            }
            let mut captured = Captured::vacant();
            captured.rows = (1..=2)
                .map(|sequence| rows::VariantChangeRow {
                    sequence,
                    hour,
                    variant: 37,
                    snapshot: Some(snapshot.clone()),
                    ..Default::default()
                })
                .collect();
            if hour == 4 {
                captured.rows[0].snapshot = None;
            }
            captured.outcome = Outcome::Rows { len: 2 };
            let projection = Projection::new(&optimized, 1, hour, force, &scratch);
            assert!(
                Projection::new(&optimized, 2, hour, force, &scratch)
                    .observed_shared_baseline::<tables::VariantDefinitions>()
                    .is_none()
            );
            assert!(
                Projection::new(&optimized, 1, hour + 1, force, &scratch)
                    .observed_shared_baseline::<tables::VariantDefinitions>()
                    .is_none()
            );
            if !force && hour == 1 {
                assert!(matches!(
                    collector_test_support::with_allocations_denied(|| {
                        project_definitions(captured.history(), Some(&projection))
                    }),
                    TableInput::SharedComplete(_)
                ));
            }
            let actual = project(&captured, Some(&projection));
            if !force && hour < 4 {
                for input in actual.iter().skip(usize::from(hour == 2)) {
                    assert!(matches!(input, TableInput::SharedComplete(rows)
                        if Arc::ptr_eq(rows, optimized.shared_baseline(rows.table()).unwrap())));
                }
            }
            let full = Projection::new(&reference, 1, hour, true, &scratch);
            let expected = project(&captured, Some(&full));
            let mut next = input(hour);
            let mut control = input(hour);
            let shared = |input| match input {
                TableInput::Sampled(TableView::Complete(rows)) => {
                    TableInput::SharedComplete(Arc::new(rows))
                }
                input => input,
            };
            for (table, (actual, expected)) in
                TABLES[3..7].iter().zip(actual.into_iter().zip(expected))
            {
                next.tables[table.index()] = shared(actual);
                control.tables[table.index()] = shared(expected);
            }
            assert_eq!(
                optimized.encode_input(next, force).unwrap(),
                reference.encode_input(control, force).unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}

fn burst_definition(id: u64, name: &[u8]) -> rows::VariantDefinitionRow {
    rows::VariantDefinitionRow {
        id,
        display_name: name.to_vec(),
        design: rows::Recorded::Value(rows::VariantDesignState::default()),
        ..Default::default()
    }
}

#[test]
fn membership_burst_replaces_stable_slots_without_shifting_the_remaining_census() {
    let baseline: Vec<_> = (1..=5_000).map(|id| burst_definition(id, b"")).collect();
    let replacements: Vec<_> = (0..8_000)
        .map(|index| {
            let mut row = burst_definition(2_500, b"");
            row.obsolete = (index % 2) as u8;
            row
        })
        .collect();
    let mut state = DefinitionState::new(&baseline).unwrap();
    state.insert(burst_definition(5_001, b"")).unwrap();
    let slot = state.positions[&2_500];
    let untouched = state.get(2_501).unwrap() as *const rows::VariantDefinitionRow;
    let storage = state.rows.as_ptr();
    collector_test_support::with_allocations_denied(|| {
        for row in replacements {
            state.remove(2_500);
            assert_eq!(state.get(2_501).unwrap() as *const _, untouched);
            state.insert(row).unwrap();
            assert_eq!(state.positions[&2_500], slot);
            assert_eq!(state.rows.as_ptr(), storage);
        }
    });
    let result = state.finish().unwrap();
    assert_eq!(result.len(), 5_001);
    assert_eq!(result[2_499].obsolete, 1);
    assert!(result.windows(2).all(|pair| pair[0].id < pair[1].id));
}

#[test]
fn membership_burst_preserves_terminal_definitions_and_every_history_row_in_frame_bytes() {
    let baseline: Vec<_> = (1..=5_000)
        .map(|id| burst_definition(id, b"base"))
        .collect();
    let snapshot = |id, name: &[u8]| rows::VariantChangeRow {
        variant: id,
        snapshot: Some(rows::VariantSnapshot {
            definition: burst_definition(id, name).into(),
            stats: vec![Some(0); 10],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut history = vec![snapshot(5_001, b"new")];
    history
        .extend((0..8_000).map(|index| snapshot(2_500, if index % 2 == 0 { b"a" } else { b"b" })));
    history.extend([
        rows::VariantChangeRow {
            variant: 2_500,
            scalar: Some(rows::VariantScalarChange {
                obsolete: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        },
        rows::VariantChangeRow {
            variant: 2_500,
            sources: Some(rows::VariantSourceReferences {
                direct: vec![7],
                named: vec![11],
            }),
            ..Default::default()
        },
        rows::VariantChangeRow {
            variant: 2_500,
            flags: Some([1, 0, 1]),
            ..Default::default()
        },
        rows::VariantChangeRow {
            variant: 3,
            removed: true,
            ..Default::default()
        },
        snapshot(3, b"republished"),
        rows::VariantChangeRow {
            organisation: Some(rows::VariantOrganisationChange {
                organisation: 79,
                live: false,
            }),
            ..Default::default()
        },
    ]);
    for (index, row) in history.iter_mut().enumerate() {
        row.sequence = index as u64 + 1;
        row.hour = 1;
    }
    let mut expected = baseline.clone();
    expected[2] = burst_definition(3, b"republished");
    expected[2_499] = burst_definition(2_500, b"b");
    expected[2_499].obsolete = 1;
    let state = expected[2_499].design.require_mut("fixture").unwrap();
    state.bonus_sources = vec![7];
    state.named_sources = vec![11];
    [
        state.can_upgrade_type,
        state.can_upgrade_variant,
        state.can_upgrade_modules,
    ] = [1, 0, 1];
    expected.push(burst_definition(5_001, b"new"));
    for force in [false, true] {
        let mut optimized = replay_frame::frame::FrameEncoder::new();
        let mut reference = replay_frame::frame::FrameEncoder::new();
        for encoder in [&mut optimized, &mut reference] {
            let mut first = input(0);
            first.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(rows::TableRows::VariantDefinitions(baseline.clone())).into();
            encoder.encode_input(first, false).unwrap();
        }
        let mut captured = Captured::vacant();
        captured.rows = history.clone();
        captured.outcome = Outcome::Rows { len: history.len() };
        let scratch = crate::projection::ProjectionScratch::default();
        let projection = Projection::new(&optimized, 1, 1, force, &scratch);
        let definitions = project_definitions(captured.history(), Some(&projection));
        assert!(
            matches!(&definitions, TableInput::Sampled(TableView::Complete(rows))
            if rows.rows::<tables::VariantDefinitions>() == Some(expected.as_slice()))
        );
        let changes = captured.table_input(Some(&projection));
        assert!(
            matches!(&changes, TableInput::Sampled(TableView::Complete(rows))
            if rows.rows::<tables::VariantChanges>() == Some(history.as_slice()))
        );
        let mut actual = input(1);
        actual.tables[TableId::VariantDefinitions.index()] = definitions;
        actual.tables[TableId::VariantChanges.index()] = changes;
        let mut wanted = input(1);
        wanted.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(rows::TableRows::VariantDefinitions(expected.clone())).into();
        wanted.tables[TableId::VariantChanges.index()] =
            TableView::Complete(rows::TableRows::VariantChanges(history.clone())).into();
        let mut actual_bytes = Vec::new();
        let mut wanted_bytes = Vec::new();
        optimized
            .encode_input(actual, force)
            .unwrap()
            .encode(&mut actual_bytes)
            .unwrap();
        reference
            .encode_input(wanted, force)
            .unwrap()
            .encode(&mut wanted_bytes)
            .unwrap();
        assert_eq!(actual_bytes, wanted_bytes);
    }
}

#[test]
fn a_flags_row_naming_flags_its_design_holds_stays_out_of_the_table() {
    let mut baseline: Vec<_> = (1..=3).map(|id| burst_definition(id, b"base")).collect();
    let state = baseline[0].design.require_mut("fixture").unwrap();
    [
        state.can_upgrade_type,
        state.can_upgrade_variant,
        state.can_upgrade_modules,
    ] = [1, 0, 1];
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut first = input(0);
    first.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(rows::TableRows::VariantDefinitions(baseline)).into();
    encoder.encode_input(first, false).unwrap();
    let touch = |variant, held| rows::VariantChangeRow {
        variant,
        flags: Some(held),
        ..Default::default()
    };
    let mut history = vec![
        touch(1, [1, 0, 1]),
        touch(1, [1, 1, 1]),
        touch(1, [1, 1, 1]),
        rows::VariantChangeRow {
            variant: 2,
            snapshot: Some(rows::VariantSnapshot {
                definition: burst_definition(2, b"new").into(),
                stats: vec![Some(0); 10],
                ..Default::default()
            }),
            ..Default::default()
        },
        touch(2, [0, 0, 0]),
        touch(3, [1, 0, 1]),
    ];
    for (index, row) in history.iter_mut().enumerate() {
        row.sequence = index as u64 + 1;
        row.hour = 1;
    }
    let mut captured = Captured::vacant();
    captured.rows = history.clone();
    captured.outcome = Outcome::Rows { len: history.len() };
    let scratch = crate::projection::ProjectionScratch::default();
    let projection = Projection::new(&encoder, 1, 1, false, &scratch);
    for (projection, kept) in [(Some(&projection), &[1, 3, 5][..]), (None, &[0, 1, 3, 5][..])] {
        let kept: Vec<_> = kept.iter().map(|&at: &usize| history[at].clone()).collect();
        assert!(
            matches!(captured.table_input(projection), TableInput::Sampled(TableView::Complete(rows))
            if rows.rows::<tables::VariantChanges>() == Some(kept.as_slice()))
        );
    }
}

#[test]
fn membership_projection_refuses_intermediate_unknown_and_incomplete_occurrences() {
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut first = input(0);
    first.tables[TableId::VariantDefinitions.index()] = TableView::Complete(
        rows::TableRows::VariantDefinitions(vec![burst_definition(1, b"base")]),
    )
    .into();
    encoder.encode_input(first, false).unwrap();
    let snapshot = |id| rows::VariantChangeRow {
        variant: id,
        snapshot: Some(rows::VariantSnapshot {
            definition: burst_definition(id, b"new").into(),
            stats: vec![Some(0); 10],
            ..Default::default()
        }),
        ..Default::default()
    };
    let scratch = crate::projection::ProjectionScratch::default();
    for force in [false, true] {
        let projection = Projection::new(&encoder, 1, 1, force, &scratch);
        for (bad, reason) in [
            (
                rows::VariantChangeRow {
                    variant: 1,
                    scalar: Some(rows::VariantScalarChange {
                        obsolete: Some(1),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                "variant scalar names an unknown design",
            ),
            (
                rows::VariantChangeRow {
                    variant: 1,
                    sources: Some(Default::default()),
                    ..Default::default()
                },
                "variant sources name an unknown design",
            ),
            (
                rows::VariantChangeRow {
                    variant: 1,
                    flags: Some([1, 0, 1]),
                    ..Default::default()
                },
                "variant flags name an unknown design",
            ),
            (
                rows::VariantChangeRow {
                    variant: 1,
                    ..Default::default()
                },
                "variant changes have an incomplete observation",
            ),
        ] {
            let history = [
                snapshot(2),
                rows::VariantChangeRow {
                    variant: 1,
                    removed: true,
                    ..Default::default()
                },
                bad,
                snapshot(1),
            ];
            assert!(
                matches!(project_definitions(snapshots::History::from_rows(&history), Some(&projection)),
                TableInput::Sampled(TableView::Unavailable { reason: actual }) if actual == reason)
            );
        }
    }
}

#[test]
fn cold_storage_warmup_keeps_the_serial_census_capacity_and_certificate_decision() {
    if !isolated() {
        return;
    }
    for count in [67, 1500] {
        let space = super::super::variant_scale_tests::space(count, 1);
        let mut registry = Vec::new();
        let outcome =
            native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry);
        let mut variants = Variants::vacant();
        variants.capture_state_carried(
            &space.memory(),
            space.base,
            Sources {
                rows: &registry,
                outcome,
            },
            7,
            true,
            false,
        );
        assert_eq!(variants.definitions.outcome.len(), registry.len());
        let mut reference = None;
        for warm in [false, true] {
            storage::release();
            held::release();
            sources::release();
            liveness::release();
            *cold_store().lock().unwrap() = Store {
                batch: Batch::empty(),
                spare: Batch::empty(),
                watched: HashMap::new(),
                published: Published::default(),
                interests: sources::Interests::default(),
                epoch: EPOCH.load(Ordering::Acquire),
            };
            SAVED_BASELINE.store(false, Ordering::Release);
            SOURCE_BASELINE.store(false, Ordering::Release);
            LIVENESS_BASELINE.store(false, Ordering::Release);
            RECOVER.store(true, Ordering::Release);
            ENABLED.store(true, Ordering::Release);
            collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
            begin_read();
            if warm {
                let mut work = cold_storage(&registry).unwrap();
                work.capture(&mut |parts| {
                    Ok(std::thread::scope(|scope| {
                        let jobs: Vec<_> = parts
                            .into_iter()
                            .map(|mut part| {
                                scope.spawn(move || {
                                    part.capture();
                                    part
                                })
                            })
                            .collect();
                        jobs.into_iter()
                            .rev()
                            .map(|job| job.join().unwrap())
                            .collect()
                    }))
                });
                assert!(!SAVED_BASELINE.load(Ordering::Acquire));
                assert!(!SOURCE_BASELINE.load(Ordering::Acquire));
                assert!(!LIVENESS_BASELINE.load(Ordering::Acquire));
            }
            prepare_partitioned(&registry, &variants, &mut |mut parts| {
                for part in &mut parts {
                    part.capture();
                }
                Ok(parts)
            });
            let store = cold_store().lock().unwrap();
            let mut watches: Vec<_> = store
                .watched
                .values()
                .map(|watch| (watch.id, watch.pointer, watch.flags))
                .collect();
            watches.sort_unstable();
            let capacities = [&store.batch, &store.spare]
                .map(|batch| (batch.events.capacity(), batch.failure));
            let certificate = (
                SAVED_BASELINE.load(Ordering::Acquire),
                SOURCE_BASELINE.load(Ordering::Acquire),
                LIVENESS_BASELINE.load(Ordering::Acquire),
                RECOVER.load(Ordering::Acquire),
                CENSUS_BASELINE.load(Ordering::Acquire),
                store.epoch,
            );
            assert!(certificate.0 && certificate.1 && certificate.2 && !certificate.3);
            assert_eq!(watches.len(), registry.len());
            let value = (watches, capacities, certificate);
            if let Some(reference) = reference.as_ref() {
                assert_eq!(&value, reference);
            } else {
                reference = Some(value);
            }
        }
    }
}

#[test]
fn events_and_watches_past_prepared_storage_grow_where_they_are_written() {
    let mut batch = Batch::empty();
    for pointer in 1..=3 {
        batch.room().unwrap();
        batch.events.push(Event {
            kind: Kind::Design,
            sequence: pointer as u64,
            hour: 0,
            pointer,
            variant: 0,
            removed: true,
            start: 0,
            end: 0,
        });
    }
    assert_eq!(batch.events.len(), 3);
    assert_eq!(batch.required_events, 3);
    let full = batch.events.capacity() - batch.events.len();
    for _ in 0..full {
        batch.events.push(batch.events[0]);
    }
    assert_eq!(
        collector_test_support::with_allocations_denied(|| batch.room()),
        Err(STORAGE_FAILED)
    );
    let mut interests = sources::Interests::default();
    for pointer in 1..=3 {
        interests.insert(sources::Key::Direct(pointer), 7).unwrap();
        interests.organisation(pointer as u64, Some(pointer)).unwrap();
    }
    assert_eq!(interests.entries.len(), 3);
    assert_eq!(interests.organisations.len(), 3);
    assert_eq!(interests.organisation_pointers.len(), 3);
}

#[test]
fn stat_timing_quiet_refuses_when_design_hooks_are_disabled() {
    if !isolated() {
        return;
    }
    ENABLED.store(false, Ordering::Release);
    assert_eq!(active(), 0);
    assert_eq!(
        stat_timing_quiet(),
        Err("variant stat writer hooks disabled")
    );
    let mut witness = StatLaneWitness;
    native::StatWitness::record_withheld(&mut witness, 4, "variant stat writer hooks disabled");
    let reported = stat_witness_snapshot();
    assert_eq!(reported.last_lane, Some("maximum_speed"));
    assert_eq!(
        reported.last_reason,
        Some("variant stat writer hooks disabled")
    );
    assert!(reported.by_lane[4] >= 1);
    assert!(reported.total_withheld >= 1);
}

#[test]
fn stat_timing_quiet_detects_design_mio_read_between_and_during_reads() {
    if !isolated() {
        return;
    }
    static DURING_WAS_QUIET: AtomicBool = AtomicBool::new(false);
    unsafe extern "system-unwind" fn original(_: usize, _: usize, _: usize, _: usize) -> usize {
        DURING_WAS_QUIET.store(stat_timing_quiet().is_ok(), Ordering::SeqCst);
        0
    }
    ENABLED.store(true, Ordering::Release);
    assert_eq!(active(), 0);
    collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
    let callback = collector_observers::world_input_changes::design_mio_read_for_test(original);
    let before = stat_timing_quiet().expect("quiet before design_mio_read");
    DURING_WAS_QUIET.store(true, Ordering::SeqCst);
    unsafe { callback(0x1000, 0, 0, 0) };
    let after = stat_timing_quiet().expect("quiet after design_mio_read");
    ENABLED.store(false, Ordering::Release);
    collector_observers::world_input_changes::set_design_watch_enabled_for_test(false);
    assert!(!DURING_WAS_QUIET.load(Ordering::SeqCst));
    assert_ne!(before, after);
}
