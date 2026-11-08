use super::*;
use engine_native::{
    ccountry, cgame_state, csub_unit_definition as subunit, memory::CountedMemory,
    memory::SliceMemory,
};

/// Base zero; the subunit database pointer reads as 0x8800.
struct Fixture {
    bytes: Vec<u8>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == subunit::DATABASE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x8800_u64.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}
fn fixture() -> Fixture {
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: u64, count: u32) {
        word(bytes, at, data);
        word(bytes, at + 8, u64::from(count) | (u64::from(count) << 32));
    }
    fn text(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
        word(bytes, at + 16, value.len() as u64);
        word(bytes, at + 24, 15);
    }
    let mut bytes = vec![0; 0x9000];
    array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 2);
    array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 2);
    word(&mut bytes, 0x2008, 0x3000);
    bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
    word(&mut bytes, 0x3000, ccountry::VTABLE_RVA.0 as u64);
    word(&mut bytes, 0x3008, 1);
    array(&mut bytes, 0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
    word(&mut bytes, 0x4000, 0x5000);
    word(&mut bytes, 0x5000, native::VTABLE_RVA.0 as u64);
    word(&mut bytes, 0x5008, 0xffff_ffff_0000_0034);
    word(&mut bytes, 0x3000 + ccountry::DEPLOYMENT_MANAGER, 0x8400);
    word(
        &mut bytes,
        0x8400,
        engine_deployment::cdeployment_status::VTABLE_RVA.0 as u64,
    );
    word(
        &mut bytes,
        0x8400 + engine_deployment::cdeployment_status::OWNER,
        0x3000,
    );
    text(&mut bytes, 0x5000 + native::NAME, &[b'A', 0, 255]);
    text(&mut bytes, 0x5000 + native::NAME_KEY, b"fallback");
    text(&mut bytes, 0x5000 + native::MODEL_ENTITY, b"GER_x_entity");
    array(&mut bytes, 0x5000 + native::SUBUNIT_COUNTS, 0x6000, 1);
    word(&mut bytes, 0x6000, 0x7000);
    word(&mut bytes, 0x6008, 3);
    word(&mut bytes, 0x5000 + native::PRIORITY_UNIT, 0x7000);
    word(&mut bytes, 0x7000, subunit::VTABLE_RVA.0 as u64);
    word(&mut bytes, 0x7000 + subunit::TOKEN, 777);
    bytes[0x7000 + subunit::IS_REAL] = 1;
    text(&mut bytes, 0x7000 + subunit::NAME, b"infantry");
    word(&mut bytes, 0x7000 + subunit::DATABASE_INDEX, 1);
    array(&mut bytes, 0x8800 + subunit::DATABASE_VECTOR, 0x8900, 2);
    word(&mut bytes, 0x8908, 0x7000);
    Fixture { bytes }
}
fn template_outcomes(captured: &Captured) -> [Outcome; 2] {
    let [dictionary, icons, _] = captured.outcomes();
    [dictionary, icons]
}

fn capacity() -> Capacities {
    Capacities {
        rows: 2,
        bytes: 32,
        counts: 16,
        variants: 4,
        icon_rows: 2,
        country_rows: 2,
        icon_bytes: 32,
        name_bytes: 32,
        database: 8,
        locals: 8,
    }
}

#[test]
fn owned_template_tails_match_synchronous_rows_and_survive_later_writer_marks() {
    let mut source = fixture();
    let changes = CoreChanges::new();
    let mut synchronous = Captured::with_capacities(&capacity());
    let mut deferred = Captured::with_capacities(&capacity());
    for hour in 0..3 {
        synchronous.capture_original(&source, 0, 0x1000, hour, false, &changes);
        deferred.capture_observed(&source, 0, 0x1000, hour, false, &changes);
        drop(changes.templates.notify(0x5000, 1));
        source.bytes[0x6008..0x600c].copy_from_slice(&(hour + 7).to_le_bytes());
        deferred.settle_owned();
        assert_eq!(deferred.tables(), synchronous.tables());
        assert_eq!(deferred.country_state(), synchronous.country_state());
        assert_eq!(deferred.model_failures(), synchronous.model_failures());
        deferred.settle_owned();
        assert_eq!(deferred.tables(), synchronous.tables());
    }
    deferred.capture_observed(&source, 0, 0x1000, 3, false, &changes);
    deferred.reset();
    assert!(!deferred.pending_owned);
    deferred.capture_original(&source, 0, 0x1000, 4, false, &changes);
    synchronous.capture_original(&source, 0, 0x1000, 4, true, &changes);
    assert_eq!(deferred.tables(), synchronous.tables());
}

#[test]
fn enabled_unsized_startup_records_known_template_and_country_values_through_spool() {
    use crate::template_changes::Changes;
    use engine_deployment::cdeployment_status as deployment;
    use engine_occupation::ccountry_occupation_status as occupation;
    use replay::{spool::Record, spool::encode_record};
    use replay_frame::{
        frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput, frame::Metrics,
    };
    const TEST: &str = "extended::division_templates::tests::enabled_unsized_startup_records_known_template_and_country_values_through_spool";
    const CHILD: &str = "BATTLE_PLAN_TEMPLATE_UNSIZED_STARTUP_TEST";
    // Exercise the actual process-wide observers without enabling them in
    // other tests. No native hooks are patched in this synthetic process.
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let changes = crate::template_changes::unsized_observer_for_tests();
    let reset = crate::local_subunit_changes::unsized_observer_for_tests();
    assert!(changes.quiet().is_some());
    assert!(
        changes.sequence(0x5000).is_none(),
        "no template watch table yet"
    );
    assert!(crate::local_subunit_changes::Changes::quiet(&reset).is_some());
    assert!(
        crate::local_subunit_changes::Changes::sequence(&reset, 0x8400).is_none(),
        "no deployment-reset watch table yet"
    );

    let mut source = fixture();
    let staff = 0xffff_ffff_0000_0034_u64;
    for (at, value) in [
        (0x5000 + native::REGIMENT_DATA, 0x6100),
        (0x5000 + native::REGIMENT_DATA + 8, (2_u64 << 32) | 2),
        (0x6100, 0x7000),
        (0x8400 + deployment::SELECTED_STAFF_TEMPLATE, staff),
        (0x3000 + ccountry::OCCUPATION_STATUS, 0x8000),
        (0x8000, occupation::VTABLE_RVA.0 as u64),
        (
            0x8000 + occupation::SECONDARY_BASE,
            occupation::SECONDARY_VTABLE_RVA.0 as u64,
        ),
    ] {
        source.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (0x5000 + native::GRID_DIMENSIONS, (2_i32 << 16) | 1),
        (0x5000 + native::PRIORITY, -7),
        (0x8400 + deployment::NEXT_HQ_ORDER, 23),
        (0x8000 + occupation::GARRISON_PRIORITY, -3),
    ] {
        source.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    source.bytes[0x3000 + ccountry::LOCK_ALL_TEMPLATES] = 1;

    // The loader's existing live sizing pass precedes hook publication and
    // the first observer boundary; there has been no capture census yet.
    let mut capacities = Capacities::default();
    capacities.presize(&source, 0, 0x1000).unwrap();
    crate::local_subunit_changes::boundary(true);
    crate::template_changes::boundary(true);
    let mut capture = Captured::with_capacities(&capacities);
    capture.capture_original(&source, 0, 0x1000, 1, true, &changes);
    let mut input = FrameInput {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|_| TableInput::Sampled(unavailable("unused test table"))),
        metrics: Metrics::default(),
    };
    for (table, value) in TABLES.into_iter().zip(capture.table_inputs()) {
        input.tables[table.index()] = value;
    }
    let mut encoder = FrameEncoder::new();
    let mut prepared = encoder.prepare_bytes(input, true).unwrap();
    let frame = prepared.take_frame().decode().unwrap();
    assert!(
        !encode_record(&Record::Frame(frame.clone()))
            .unwrap()
            .is_empty()
    );
    encoder.commit(prepared).unwrap();
    let decoded = FrameDecoder::new().apply(&frame).unwrap();
    let icons = decoded.complete::<tables::DivisionTemplateIcons>().unwrap();
    let layout = icons[0]
        .layout
        .require("layout")
        .unwrap()
        .as_ref()
        .expect("readable layout must be known at H");
    assert_eq!((layout.regiments.columns, layout.regiments.rows), (1, 2));
    assert_eq!(layout.regiments.cells.len(), 2);
    assert_eq!(layout.regiments.cells[0].token, 777);
    assert!(layout.regiments.cells[0].occupied);
    assert!(!layout.regiments.cells[1].occupied);
    let settings = icons[0]
        .settings
        .require("settings")
        .unwrap()
        .as_ref()
        .expect("readable settings must be known at H");
    assert_eq!(settings.priority, -7);
    let countries = decoded
        .complete::<tables::CountryTemplateState>()
        .expect("readable country configuration must be known at H");
    assert_eq!(countries.len(), 1);
    assert_eq!(countries[0].selected_staff, staff);
    assert_eq!(countries[0].next_hq_order, 23);
    assert_eq!(countries[0].lock_all, 1);
    assert_eq!(countries[0].garrison_priority, Recorded::Value(Some(-3)));
    assert!(matches!(countries[0].garrisons, Recorded::Value(Some(_))));
}

#[test]
fn failed_template_observer_preserves_fresh_template_and_country_values() {
    use crate::hook_failures::{Family, FamilyHooks};
    use crate::template_changes::{Changes, Observer};
    use engine_deployment::cdeployment_status as deployment;
    use engine_occupation::ccountry_occupation_status as occupation;
    use replay_frame::frame::{FrameInput, Metrics};

    const TEST: &str = "extended::division_templates::tests::failed_template_observer_preserves_fresh_template_and_country_values";
    const CHILD: &str = "BATTLE_PLAN_TEMPLATE_INSTALL_FAILURE_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}{}",
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    collector_primitives::build_gate::record(true, None);
    let hooks = FamilyHooks::<crate::template_changes::Hooks>::construct(0, || {
        Err("synthetic nearby relay allocation failure")
    }).unwrap();
    assert!(hooks.inner.is_none());
    assert!(crate::hook_failures::failed(Family::Templates));
    assert!(!crate::hook_failures::failed(Family::ArmyStats));
    assert!(Observer.fresh_only());
    assert_eq!(Observer.quiet(), None);
    assert_eq!(Observer.sequence(0x5000), None);

    let mut source = fixture();
    for (at, value) in [
        (0x5000 + native::REGIMENT_DATA, 0x6100),
        (0x5000 + native::REGIMENT_DATA + 8, (2_u64 << 32) | 2),
        (0x6100, 0x7000),
        (0x3000 + ccountry::OCCUPATION_STATUS, 0x8000),
        (0x8000, occupation::VTABLE_RVA.0 as u64),
        (0x8000 + occupation::SECONDARY_BASE, occupation::SECONDARY_VTABLE_RVA.0 as u64),
    ] {
        source.bytes[at..][..8].copy_from_slice(&value.to_le_bytes());
    }
    source.bytes[0x5000 + native::GRID_DIMENSIONS..][..4]
        .copy_from_slice(&((2_u32 << 16) | 1).to_le_bytes());
    let mut captured = Captured::with_capacities(&capacity());
    for (hour, key, priority) in [(1, b"first".as_slice(), -7_i32), (2, b"later".as_slice(), 11)] {
        set_name(&mut source, b"", key);
        source.bytes[0x5000 + native::PRIORITY..][..4].copy_from_slice(&priority.to_le_bytes());
        source.bytes[0x8000 + occupation::GARRISON_PRIORITY..][..4]
            .copy_from_slice(&priority.to_le_bytes());
        source.bytes[0x8400 + deployment::NEXT_HQ_ORDER..][..4]
            .copy_from_slice(&priority.to_le_bytes());
        captured.capture_original(&source, 0, 0x1000, hour, false, &Observer);
        let mut input = FrameInput {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| TableView::Complete(
                replay_data::rows::TableRows::empty(TableId::ALL[index])).into()),
        };
        for (table, value) in TABLES.into_iter().zip(captured.table_inputs()) {
            input.tables[table.index()] = value;
        }
        crate::hook_failures::apply(&mut input).unwrap();
        let TableInput::Sampled(TableView::Complete(rows)) = &input.tables[TABLES[0].index()] else {
            panic!("readable dictionary was discarded");
        };
        assert_eq!(rows.rows::<tables::DivisionTemplates>().unwrap()[0].name_key,
            Recorded::Value(key.to_vec()));
        let TableInput::Sampled(TableView::Complete(rows)) = &input.tables[TABLES[1].index()] else {
            panic!("readable template was discarded");
        };
        let row = &rows.rows::<tables::DivisionTemplateIcons>().unwrap()[0];
        let settings = row.settings.require("settings").unwrap().as_ref().unwrap();
        assert_eq!(settings.priority, priority);
        assert!(row.layout.require("layout").unwrap().is_some());
        // Complete country rows are published shared with the census snapshot.
        let rows = match &input.tables[TABLES[2].index()] {
            TableInput::Sampled(TableView::Complete(rows)) => rows,
            TableInput::SharedComplete(rows) => rows.as_ref(),
            _ => panic!("readable country configuration was discarded"),
        };
        let row = &rows.rows::<tables::CountryTemplateState>().unwrap()[0];
        assert_eq!(row.next_hq_order, priority);
        assert_eq!(row.garrison_priority, Recorded::Value(Some(priority)));
        assert!(matches!(&input.tables[TableId::OrdersGroupCounterInputs.index()],
            TableInput::Sampled(TableView::Complete(_))));
    }
    source.bytes[0x5000 + native::SUBUNIT_COUNTS..][..8]
        .copy_from_slice(&0xf000_u64.to_le_bytes());
    captured.capture_original(&source, 0, 0x1000, 3, false, &Observer);
    assert_eq!(displayed(captured.table_inputs()).model_unreadable, 1);
}

#[test]
fn cold_and_post_gap_template_columns_pass_spool_validation_with_named_refusals() {
    use crate::template_changes::Watches;
    use replay::{spool::Record, spool::encode_record};
    use replay_data::{rows::RowCodec, rows::TableRows};
    use replay_frame::{
        frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput, frame::Metrics,
    };

    fn admit(
        capture: &Captured,
        encoder: &mut FrameEncoder,
        hour: i32,
    ) -> replay_frame::frame::DecodedFrame {
        let mut input = FrameInput {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| TableInput::Sampled(unavailable("unused test table"))),
            metrics: Metrics::default(),
        };
        for (table, value) in TABLES.into_iter().zip(capture.table_inputs()) {
            input.tables[table.index()] = value;
        }
        // This is the production spool's validation/prepare/commit path, not
        // an unchecked row codec. Every present column must be encodable.
        let mut prepared = encoder.prepare_bytes(input, true).unwrap();
        let frame = prepared.take_frame().decode().unwrap();
        assert!(frame.keyframe);
        let mut wire = Vec::new();
        Record::Frame(frame.clone()).encode(&mut wire).unwrap();
        assert!(
            !encode_record(&Record::Frame(frame.clone()))
                .unwrap()
                .is_empty()
        );
        encoder.commit(prepared).unwrap();
        let decoded = FrameDecoder::new().apply(&frame).unwrap();
        for table in [
            TableId::DivisionTemplateIcons,
            TableId::CountryTemplateState,
        ] {
            match &*decoded.tables[table.index()] {
                TableView::Complete(rows) => {
                    rows.validate_keys().unwrap();
                    match rows {
                        TableRows::DivisionTemplateIcons(rows) => {
                            for row in rows {
                                assert_eq!(row.not_recorded_fields(), 0);
                            }
                        }
                        TableRows::CountryTemplateState(rows) => {
                            for row in rows {
                                assert_eq!(row.not_recorded_fields(), 0);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                TableView::Unavailable { reason } => assert!(!reason.is_empty()),
                _ => panic!("fixture must be complete or explicitly refused"),
            }
        }
        decoded
    }

    for post_gap in [false, true] {
        for case in [
            "ready",
            "template writer",
            "country writer",
            "unobserved",
            "model failure",
            "empty roster",
            "garrisons ready",
            "garrison writer",
            "deployment failure",
        ] {
            let watches = Watches::enabled(32);
            let unobserved = Watches::new();
            let mut source = fixture();
            let mut capture = Captured::with_capacities(&capacity());
            let mut encoder = FrameEncoder::new();
            if post_gap {
                capture.capture_original(&source, 0, 0x1000, 0, true, &watches);
                admit(&capture, &mut encoder, 0);
                capture.fail_capture(1, false, "synthetic spool gap");
                encoder = FrameEncoder::new();
            }
            if case == "model failure" {
                source.bytes
                    [0x5000 + native::SUBUNIT_COUNTS + 8..0x5000 + native::SUBUNIT_COUNTS + 12]
                    .copy_from_slice(&1_u32.to_le_bytes());
                source.bytes
                    [0x5000 + native::SUBUNIT_COUNTS + 12..0x5000 + native::SUBUNIT_COUNTS + 16]
                    .copy_from_slice(&2_u32.to_le_bytes());
            }
            if case == "empty roster" {
                source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + 12
                    ..0x3000 + ccountry::DIVISION_TEMPLATES + 16]
                    .fill(0);
            }
            if matches!(case, "garrisons ready" | "garrison writer") {
                use engine_occupation::ccountry_occupation_status as occupation;
                for (at, value) in [
                    (0x3000 + ccountry::OCCUPATION_STATUS, 0x8000_u64),
                    (0x8000, occupation::VTABLE_RVA.0 as u64),
                    (
                        0x8000 + occupation::SECONDARY_BASE,
                        occupation::SECONDARY_VTABLE_RVA.0 as u64,
                    ),
                ] {
                    source.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
                }
            }
            if case == "deployment failure" {
                source.bytes[0x8400..0x8408].fill(0);
            }
            let writer = match case {
                "template writer" => Some(watches.notify(0x5000, 0)),
                "country writer" => Some(watches.notify(1, 0)),
                "garrison writer" => Some(watches.notify(0x8000, 0)),
                _ => None,
            };
            let changes = if case == "unobserved" {
                &unobserved
            } else {
                &watches
            };
            capture.capture_original(&source, 0, 0x1000, 2, true, changes);
            let decoded = admit(&capture, &mut encoder, 2);
            let icons = decoded.complete::<tables::DivisionTemplateIcons>().unwrap();
            if case == "empty roster" {
                assert!(icons.is_empty());
            } else {
                // This observer certifies only a quiet writer domain. Any
                // active writer refuses both template and country reads.
                let refused = matches!(
                    case,
                    "template writer" | "country writer" | "garrison writer" | "unobserved"
                );
                assert_eq!(
                    matches!(icons[0].settings, Recorded::Value(None)),
                    refused,
                    "{case}"
                );
                assert_eq!(
                    capture
                        .model_failures()
                        .reasons
                        .iter()
                        .any(|(reason, count)| *reason
                            == "template details writer certificate unavailable"
                            && *count == 1),
                    refused,
                    "certificate refusal must remain named and counted: {case}"
                );
                assert_eq!(
                    matches!(icons[0].layout, Recorded::Value(None)),
                    refused || case == "model failure",
                    "{case}"
                );
                assert!(matches!(icons[0].display_name, Recorded::Value(Some(_))));
            }
            if matches!(
                case,
                "template writer" | "country writer" | "garrison writer" | "unobserved"
            ) {
                assert!(
                    matches!(&*decoded.tables[TableId::CountryTemplateState.index()],
                    TableView::Unavailable { reason } if reason.contains("country template writer certificate"))
                );
            } else if case == "deployment failure" {
                assert!(
                    matches!(&*decoded.tables[TableId::CountryTemplateState.index()],
                    TableView::Unavailable { reason } if reason.contains("deployment"))
                );
            } else {
                let countries = decoded.complete::<tables::CountryTemplateState>().unwrap();
                assert_eq!(countries.len(), 1);
                // Missing occupation input refuses only its own fields.
                if case == "garrisons ready" {
                    assert!(matches!(
                        countries[0].garrison_priority,
                        Recorded::Value(Some(_))
                    ));
                    assert!(matches!(countries[0].garrisons, Recorded::Value(Some(_))));
                } else {
                    assert_eq!(countries[0].garrison_priority, Recorded::Value(None));
                    assert_eq!(countries[0].garrisons, Recorded::Value(None));
                }
            }
            drop(writer);
            capture.capture_original(&source, 0, 0x1000, 3, false, &watches);
            let retry = admit(&capture, &mut encoder, 3);
            if case != "deployment failure" {
                assert_eq!(
                    retry
                        .complete::<tables::CountryTemplateState>()
                        .unwrap()
                        .len(),
                    1
                );
            }
            if case != "empty roster" {
                assert!(matches!(
                    retry.complete::<tables::DivisionTemplateIcons>().unwrap()[0].settings,
                    Recorded::Value(Some(_))
                ));
            }
        }
    }
}

#[test]
fn country_template_cache_quiet_hours_read_and_allocate_nothing_and_membership_is_exact() {
    use crate::template_changes::Watches;
    use engine_occupation::ccountry_occupation_status as occupation;
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("quiet country template cache must make no native reads");
        }
    }
    let mut source = fixture();
    let mut put =
        |at: usize, value: u64| source.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    put(0x3000 + ccountry::OCCUPATION_STATUS, 0x8000);
    put(0x8000, occupation::VTABLE_RVA.0 as u64);
    put(
        0x8000 + occupation::SECONDARY_BASE,
        occupation::SECONDARY_VTABLE_RVA.0 as u64,
    );
    let country = ccountry::Country {
        pointer: 0x3000,
        canonical_slot: 1,
        id: 1,
    };
    let watches = Watches::enabled(32);
    let mut store = countries::Store::default();
    store.begin();
    let first = store.read(&source, 0, 0x1000, country, &watches).unwrap();
    assert!(first.garrison_failure.is_none());
    store.finish();
    assert!(store.changed);
    for _ in 0..3 {
        store.begin();
        crate::test_support::with_allocations_denied(|| {
            let row = store
                .read(&Unreadable, 0, 0x1000, country, &watches)
                .unwrap();
            assert!(std::sync::Arc::ptr_eq(
                first.garrisons.as_ref().unwrap(),
                row.garrisons.as_ref().unwrap()
            ));
            store.finish();
            assert!(!store.changed);
        });
    }
    // A writer that leaves the country's rows as they were changes nothing.
    drop(watches.notify(0x8400, 34));
    store.begin();
    store.read(&source, 0, 0x1000, country, &watches).unwrap();
    store.finish();
    assert!(!store.changed);
    drop(watches.notify(0x8000, 36));
    store.begin();
    crate::test_support::with_allocations_denied(|| {
        let equal = store.read(&source, 0, 0x1000, country, &watches).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            first.garrisons.as_ref().unwrap(),
            equal.garrisons.as_ref().unwrap()
        ));
    });
    store.finish();
    assert!(!store.changed);
    source.bytes
        [0x8000 + occupation::GARRISON_PRIORITY..0x8000 + occupation::GARRISON_PRIORITY + 4]
        .copy_from_slice(&(-3_i32).to_le_bytes());
    drop(watches.notify(country.id as usize, 55));
    store.begin();
    let reprioritized = store.read(&source, 0, 0x1000, country, &watches).unwrap();
    assert_eq!(reprioritized.garrison_priority, Some(-3));
    assert!(std::sync::Arc::ptr_eq(
        first.garrisons.as_ref().unwrap(),
        reprioritized.garrisons.as_ref().unwrap()
    ));
    store.finish();
    assert!(store.changed);
    // A removal changes membership, and restoring a previously cached country
    // also changes it even when no native writer touched that country's state.
    store.begin();
    store.finish();
    assert!(store.changed);
    store.begin();
    store
        .read(&Unreadable, 0, 0x1000, country, &watches)
        .unwrap();
    store.finish();
    assert!(store.changed);
    // An observed failure cannot revive a formerly held configuration, and
    // quiet hours must not retry its native reads.
    drop(watches.notify(0x8400, 34));
    source.bytes[0x8400..0x8408].fill(0);
    store.begin();
    assert!(store.read(&source, 0, 0x1000, country, &watches).is_err());
    assert!(
        store
            .read(&Unreadable, 0, 0x1000, country, &watches)
            .is_err()
    );
}

/// Both tables are read every hour, the dictionary from the icon walk, and
/// unreadable memory fails both rather than claiming an empty roster.
#[test]
fn every_hour_reads_both_tables_and_bad_memory_cannot_claim_empty() {
    let mut capture = Captured::with_capacities(&capacity());
    let present = CountedMemory::new(fixture());
    let missing = CountedMemory::new(SliceMemory {
        base: 0,
        bytes: &[],
    });
    for (hour, force) in (-24..24).map(|hour| (hour, false)).chain([(0, true)]) {
        assert!(replay_data::cadence::should_walk(TABLES[0], hour, force));
        let reads = crate::read_accounting::Capture::new();
        capture.capture(&present, 0, 0x1000, hour, force);
        let read = reads.finish();
        assert!(read[TABLES[1].index()].spans > 0);
        // The walk reads the literal and its key; the dictionary reads nothing more.
        assert_eq!(read[TABLES[0].index()].spans, 0, "{hour}");
        assert_eq!(
            template_outcomes(&capture),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        capture.capture(&missing, 0, 0x1000, hour, force);
        let [Outcome::Unavailable { reason }, Outcome::Unavailable { .. }] =
            template_outcomes(&capture)
        else {
            panic!("unreadable templates must fail at hour {hour}");
        };
        assert_ne!(reason, replay_data::cadence::SKIPPED_REASON);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
}

/// The hourly job reads the icon and demanded names directly, then publishes
/// both tables from that walk; taking its preacquisition performs no live read.
#[test]
fn hourly_template_job_publishes_direct_walk_rows() {
    use crate::acquisition_jobs::TemplatesRequest;
    use engine_native::country_memory::Cache;
    use std::sync::Arc;

    let mut source = fixture();
    let mut capture = Captured::vacant();
    for (hour, literal, custom_icon) in [(7, &b"Alpha"[..], 11_i32), (8, &b"Bravo"[..], 22)] {
        set_name(&mut source, literal, b"fallback");
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4]
            .copy_from_slice(&custom_icon.to_le_bytes());
        let mut countries = Arc::new(Cache::try_with_capacity(256).unwrap());
        let countries = Cache::prepare_shared(&mut countries, &source, 0, 0x1000).unwrap();
        let mut request = TemplatesRequest::take(&countries, 0, 0x1000, &mut capture, hour, false);
        request.run(&source);
        request.restore(&mut capture, None);
        // An unreadable source cannot affect the already acquired tables.
        capture.capture(
            &SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0x1000,
            hour,
            false,
        );
        assert_eq!(template_outcomes(&capture), [Outcome::Rows { len: 1 }; 2]);
        let [
            TableView::Complete(replay_data::rows::TableRows::DivisionTemplates(dictionary)),
            TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
        ] = capture.tables()
        else {
            panic!("complete template job tables at {hour}");
        };
        assert_eq!(dictionary.len(), 1);
        assert_eq!(dictionary[0].name, literal);
        assert_eq!(
            dictionary[0].name_key,
            Recorded::Value(b"fallback".to_vec())
        );
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].template, dictionary[0].template);
        assert_eq!(icons[0].custom_icon, custom_icon);
        assert_eq!(icons[0].priority_unit_name, b"infantry");
        assert_eq!(
            icons[0]
                .display_name_inputs(hour, Some(&dictionary[0]))
                .unwrap()
                .literal,
            literal
        );
    }
}

#[test]
fn a_failed_capture_counts_no_model_failures() {
    let mut capture = Captured::with_capacities(&capacity());
    capture
        .model_failures
        .add("template grouped subunits unreadable");
    assert_eq!(capture.model_failures().rows, 1);
    capture.fail_capture(3, false, "slow acquisition failed");
    assert_eq!(capture.model_failures(), ModelFailures::default());
}

#[test]
fn edit_finishing_between_fixed_record_copy_and_demand_cannot_certify_old_settings() {
    use crate::template_changes::Watches;
    struct Editing<'a> {
        source: Fixture,
        watches: &'a Watches,
        edited: std::cell::Cell<bool>,
    }
    impl Memory for Editing<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let copied = self.source.copy(at, out);
            if at == 0x5000 + native::REGIMENT_DATA && !self.edited.replace(true) {
                drop(self.watches.notify(0x5000, 22));
            }
            copied
        }
    }
    let watches = Watches::enabled(32);
    let source = Editing {
        source: fixture(),
        watches: &watches,
        edited: false.into(),
    };
    let mut capture = Captured::with_capacities(&capacity());
    capture.capture_original(&source, 0, 0x1000, 1, true, &watches);
    assert!(source.edited.get());
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = capture.tables()
    else {
        panic!("icons");
    };
    assert!(matches!(icons[0].settings, Recorded::Value(None)));
    capture.capture_original(&source, 0, 0x1000, 2, false, &watches);
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = capture.tables()
    else {
        panic!("icons");
    };
    assert!(matches!(icons[0].settings, Recorded::Value(Some(_))));
}

#[test]
fn conversion_preserves_exact_bytes_and_rejects_every_invalid_span() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    source.bytes
        [0x5000 + native::FORMATION_DIVISION_COUNT..0x5000 + native::FORMATION_DIVISION_COUNT + 4]
        .copy_from_slice(&(-17_i32).to_le_bytes());
    // Two regiment cells and an empty one, no support, one regimental
    // support cell, all holding the same definition.
    let mut word = |at: usize, value: u64| {
        source.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    for (grid, data, cells) in [
        (native::REGIMENT_DATA, 0x6100, &[0x7000, 0, 0x7000][..]),
        (native::REGIMENTAL_SUPPORT_DATA, 0x6200, &[0x7000][..]),
    ] {
        word(0x5000 + grid, data);
        let count = cells.len() as u64;
        word(0x5000 + grid + 8, count | (count << 32));
        for (index, &cell) in cells.iter().enumerate() {
            word(data as usize + index * 8, cell);
        }
    }
    source.bytes[0x5000 + native::GRID_DIMENSIONS..0x5000 + native::GRID_DIMENSIONS + 4]
        .copy_from_slice(&(1_u32 | (3 << 16)).to_le_bytes());
    source.bytes[0x5000 + native::REGIMENTAL_SUPPORT_DATA - 8
        ..0x5000 + native::REGIMENTAL_SUPPORT_DATA - 4]
        .copy_from_slice(&(1_u32 | (1 << 16)).to_le_bytes());
    let watches = crate::template_changes::Watches::enabled(32);
    capture.capture_original(&source, 0, 0x1000, 3, true, &watches);
    let [
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplates(rows)),
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = capture.tables()
    else {
        panic!("complete templates");
    };
    assert_eq!(rows[0].template, 0xffff_ffff_0000_0034);
    assert_eq!(rows[0].name, &[b'A', 0, 255]);
    assert_eq!(*rows[0].name_key.require("name_key").unwrap(), b"fallback");
    assert_eq!(icons[0].model_entity, b"GER_x_entity");
    assert_eq!(
        (icons[0].representative, icons[0].representative_local),
        (777, 0)
    );
    assert_eq!(
        icons[0].subunits,
        [TemplateSubunit {
            token: 777,
            local: 0,
            count: 3
        }]
    );
    let entry = |count| TemplateSubunit {
        token: 777,
        local: 0,
        count,
    };
    assert_eq!(
        icons[0].grids,
        Recorded::Value(TemplateGrids {
            regiments: vec![entry(2)],
            support: Vec::new(),
            regimental_support: vec![entry(1)],
        })
    );
    let layout = icons[0].layout.require("layout").unwrap().as_ref().unwrap();
    assert_eq!((layout.regiments.columns, layout.regiments.rows), (1, 3));
    assert_eq!(
        layout
            .regiments
            .cells
            .iter()
            .map(|cell| (cell.token, cell.occupied))
            .collect::<Vec<_>>(),
        [(777, true), (0, false), (777, true)]
    );
    assert_eq!(
        (
            layout.regimental_support.columns,
            layout.regimental_support.rows
        ),
        (1, 1)
    );
    assert_eq!(layout.regimental_support.cells.len(), 1);
    assert_eq!(icons[0].template, 0xffff_ffff_0000_0034);
    assert_eq!(icons[0].priority_unit, 777);
    assert_eq!(icons[0].priority_unit_name, b"infantry");
    assert_eq!(
        icons[0].formation_division_count,
        replay_data::rows::Recorded::Value(Some(-17))
    );
    // A row whose model inputs failed publishes the flag, keeps its
    // counter fields and is counted by reason.
    let good = capture.icon_rows.rows[0];
    capture.icon_rows.rows[0].model_failure = Some("template grouped subunits unreadable");
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(flagged)),
    ] = capture.tables()
    else {
        panic!("complete icons");
    };
    assert_eq!(flagged[0].model_unreadable, 1);
    assert_eq!(flagged[0].priority_unit_name, b"infantry");
    assert_eq!(
        flagged[0].formation_division_count,
        replay_data::rows::Recorded::Value(Some(-17))
    );
    capture.icon_rows.rows[0] = good;
    let mut failures = ModelFailures::default();
    for reason in ["a", "b", "a"] {
        failures.add(reason);
    }
    assert_eq!(
        (failures.rows, &failures.reasons[..3], failures.other),
        (3, &[("a", 2), ("b", 1), ("", 0)][..], 0)
    );
    let daily = capture.rows.rows[0];
    for name_key in [false, true] {
        capture.rows.rows[0] = daily;
        let row = &mut capture.rows.rows[0];
        let name = if name_key {
            &mut row.name_key
        } else {
            &mut row.name
        };
        *name = native::Span {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
    let original = capture.icon_rows.rows[0];
    for which in 0..7 {
        capture.icon_rows.rows[0] = original;
        let row = &mut capture.icon_rows.rows[0];
        let span = match which {
            0 => &mut row.priority_unit_name,
            1 => &mut row.model_entity,
            2 => &mut row.counts,
            3..=5 => &mut row.grids[which - 3],
            _ => &mut row.excluded_variants,
        };
        *span = native::Span {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
}

#[test]
#[ignore = "synthetic template fallback capture and encoded delta costs"]
fn border_effect_template_key_scale_bench() {
    use replay_data::rows::RowCodec;
    use std::time::Instant;

    for templates in [193, 1_500, 5_000] {
        let list = 0xa000;
        let objects = (list + templates * 8 + 15) & !15;
        let mut memory = fixture();
        let template = memory.bytes[0x5000..0x5240].to_vec();
        memory.bytes.resize(objects + templates * 0x240, 0);
        memory.bytes[0x3000 + ccountry::DIVISION_TEMPLATES..][..8]
            .copy_from_slice(&(list as u64).to_le_bytes());
        for offset in [8, 12] {
            memory.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + offset..][..4]
                .copy_from_slice(&(templates as u32).to_le_bytes());
        }
        for index in 0..templates {
            let object = objects + index * 0x240;
            memory.bytes[object..object + 0x240].copy_from_slice(&template);
            memory.bytes[list + index * 8..][..8].copy_from_slice(&(object as u64).to_le_bytes());
            memory.bytes[object + 8..][..8].copy_from_slice(
                &(((index as u64 + 1) << 32) | u64::from(native::REFERENCE_TYPE)).to_le_bytes(),
            );
        }
        let mut capacities = capacity();
        capacities.rows = templates;
        capacities.icon_rows = templates;
        capacities.bytes = templates * 16;
        capacities.icon_bytes = templates * 32;
        capacities.counts = templates;
        let mut captured = Captured::with_capacities(&capacities);
        let rows = |captured: &Captured| {
            let TableView::Complete(replay_data::rows::TableRows::DivisionTemplates(rows)) =
                captured.tables().into_iter().next().unwrap()
            else {
                panic!("complete template dictionary");
            };
            rows
        };
        for (shape, changed) in [("quiet", 0), ("typical", 1), ("all_changed", templates)] {
            captured.capture(&memory, 0, 0x1000, 3, true);
            let mut previous = rows(&captured);
            let mut times = Vec::new();
            let mut delta_sizes = Vec::new();
            for pass in 0..5 {
                for index in 0..changed {
                    memory.bytes[objects + index * 0x240 + native::NAME_KEY + 7] = b'0' + pass;
                }
                let started = Instant::now();
                captured.capture(&memory, 0, 0x1000, 3, true);
                times.push(started.elapsed());
                let current = rows(&captured);
                let mut bytes = Vec::new();
                let mut key = 0;
                for (row, old) in current.iter().zip(&previous) {
                    if row.encode_change(Some(old), key, &mut bytes).unwrap() {
                        assert_eq!(row.delta(Some(old)).unwrap().unwrap().mask, 1 << 3);
                        key = row.key();
                    }
                }
                previous = current;
                delta_sizes.push(bytes.len());
            }
            times.sort_unstable();
            assert!(delta_sizes.iter().all(|size| *size == delta_sizes[0]));
            let counted = CountedMemory::new(memory);
            let accounting = crate::read_accounting::Capture::new();
            captured.capture(&counted, 0, 0x1000, 3, true);
            let requests = accounting.finish()[TABLES[0].index()];
            println!(
                "TEMPLATE_KEY_BENCH templates={templates} shape={shape} whole_capture_p50_us={} dictionary_requests={requests:?} added_header_bytes={} prior_requested_bytes={} baseline_delta_bytes=0 delta_bytes={} initial_field_bytes={}",
                times[times.len() / 2].as_micros(),
                templates * 32,
                requests.bytes.saturating_sub((templates * 32) as u64),
                delta_sizes[0],
                templates * 12
            );
            memory = counted.0;
        }
    }
}

fn displayed(inputs: [TableInput; N]) -> DivisionTemplateIconRow {
    let [
        _,
        TableInput::Sampled(TableView::Complete(
            replay_data::rows::TableRows::DivisionTemplateIcons(mut rows),
        )),
        _,
    ] = inputs
    else {
        panic!("complete hourly icon fixture")
    };
    rows.remove(0)
}
fn set_name(source: &mut Fixture, literal: &[u8], key: &[u8]) {
    for (at, text) in [
        (0x5000 + native::NAME, literal),
        (0x5000 + native::NAME_KEY, key),
    ] {
        assert!(text.len() <= 15);
        source.bytes[at..at + 16].fill(0);
        source.bytes[at..at + text.len()].copy_from_slice(text);
        source.bytes[at + 16..at + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
        source.bytes[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
    }
}

#[test]
fn hourly_equality_and_same_day_rename_use_exact_same_frame_bytes() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    capture.capture(&source, 0, 0x1000, 9, true);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::SameAsDivisionTemplates,
            ..
        }))
    ));
    capture.capture(&source, 0, 0x1000, 10, false);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            sample_hour: replay_data::rows::PacketHour::Host,
            value: TemplateNameValue::SameAsDivisionTemplates
        }))
    ));
    // A rename reaches the same hour's dictionary, so the name certifies
    // against it.
    set_name(&mut source, b"Renamed", b"unused");
    capture.capture(&source, 0, 0x1000, 11, false);
    let [
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplates(dictionary)),
        _,
    ] = capture.tables()
    else {
        panic!("renamed dictionary")
    };
    assert_eq!(dictionary[0].name, b"Renamed");
    let renamed = displayed(capture.table_inputs());
    assert_eq!(
        renamed
            .display_name_inputs(11, Some(&dictionary[0]))
            .unwrap()
            .literal,
        b"Renamed"
    );
    assert_eq!(renamed.priority_unit_name, b"infantry");
}

/// The dictionary is complete at every hour, so a display name equal to it
/// is a same-frame certificate; a failed dictionary needs a self-contained
/// override.
#[test]
fn every_hour_certifies_against_the_same_frame_and_a_failed_dictionary_overrides() {
    let mut capture = Captured::with_capacities(&capacity());
    let source = fixture();
    capture.capture(&source, 0, 0x1000, 10, false);
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::SameAsDivisionTemplates,
            ..
        }))
    ));
    let mut small = capacity();
    small.rows = 0;
    let mut capture = Captured::with_capacities(&small);
    capture.capture(&source, 0, 0x1000, 9, true);
    capture.rows.outcome = Outcome::Unavailable {
        reason: "synthetic dictionary failure",
    };
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::Changed { .. },
            ..
        }))
    ));
}

#[test]
fn name_arena_growth_keeps_the_first_heading_and_counts_exact_demand() {
    let mut size = capacity();
    size.name_bytes = 0;
    let mut capture = Captured::with_capacities(&size);
    capture.capture(&fixture(), 0, 0x1000, 9, true);
    let row = displayed(capture.table_inputs());
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
    assert_eq!(row.priority_unit_name, b"infantry");
    assert_eq!(row.model_unreadable, 0);
    // The literal and the dictionary's key behind it.
    assert_eq!(capture.name_bytes.required, 3 + 8);
    capture.grow(&mut size);
    assert!(size.name_bytes >= 3 + 8);
}

fn large_content(countries: usize, per_country: usize) -> Fixture {
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: usize) {
        word(bytes, at, data as u64);
        word(bytes, at + 8, (count as u64) | ((count as u64) << 32));
    }
    let mut source = fixture();
    let template = source.bytes[0x5000..0x5250].to_vec();
    let slots = countries + 1;
    let registry = 0x10000;
    let links = registry + slots * 8;
    let owners = (links + slots * 4).next_multiple_of(16);
    let rosters = owners + countries * 0x200;
    let templates = rosters + countries * per_country * 8;
    source
        .bytes
        .resize(templates + countries * per_country * template.len(), 0);
    array(
        &mut source.bytes,
        0x1000 + cgame_state::COUNTRIES,
        registry,
        slots,
    );
    array(
        &mut source.bytes,
        0x1000 + cgame_state::COUNTRY_LINKS,
        links,
        slots,
    );
    for country in 0..countries {
        let id = country + 1;
        let owner = owners + country * 0x200;
        word(&mut source.bytes, registry + id * 8, owner as u64);
        source.bytes[links + id * 4..links + id * 4 + 4]
            .copy_from_slice(&(id as u32).to_le_bytes());
        word(&mut source.bytes, owner, ccountry::VTABLE_RVA.0 as u64);
        word(&mut source.bytes, owner + ccountry::ID, id as u64);
        let roster = rosters + country * per_country * 8;
        array(
            &mut source.bytes,
            owner + ccountry::DIVISION_TEMPLATES,
            roster,
            per_country,
        );
        for ordinal in 0..per_country {
            let index = country * per_country + ordinal;
            let at = templates + index * template.len();
            source.bytes[at..at + template.len()].copy_from_slice(&template);
            word(
                &mut source.bytes,
                at + native::TYPE,
                ((index as u64 + 1) << 32) | native::REFERENCE_TYPE as u64,
            );
            word(&mut source.bytes, roster + ordinal * 8, at as u64);
        }
    }
    source
}

#[test]
fn promoted_template_worker_matches_direct_rows_and_faults() {
    use crate::acquisition_jobs::{Batch, Job, Pool, Report, Role, TemplatesRequest};
    use engine_native::country_memory::Cache;
    use std::sync::Arc;

    fn word(source: &Fixture, at: usize) -> usize {
        u64::from_le_bytes(source.bytes[at..at + 8].try_into().unwrap()) as usize
    }

    assert_eq!(Role::Templates.priority(), Role::Countries.priority());
    assert_eq!(Role::Templates.priority(), Role::IntelResidual.priority());
    for countries in [1, 40] {
        for fault in 0..3 {
            let mut source = large_content(countries, 1);
            let owner = word(&source, 0x10000 + countries * 8);
            let roster = word(&source, owner + ccountry::DIVISION_TEMPLATES);
            let template = word(&source, roster);
            match fault {
                // A late optional model refusal keeps the dictionary and icons.
                1 => source.bytes[template + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                // A late roster refusal must keep the original first reason.
                2 => source.bytes[owner + ccountry::DIVISION_TEMPLATES..][..8]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
                _ => {}
            }
            let source = Arc::new(source);
            let mut direct = Captured::with_capacities(&capacity());
            let mut captured = Captured::with_capacities(&capacity());
            let pool = {
                let source = Arc::clone(&source);
                Pool::with_test_runner(1, move |job| match job {
                    Job::Templates(request) => Report::Templates(request.run(source.as_ref())),
                    _ => panic!("only Templates was submitted"),
                })
            };
            for hour in [7, 8] {
                direct.capture(source.as_ref(), 0, 0x1000, hour, false);
                let mut cache = Arc::new(Cache::try_with_capacity(countries + 1).unwrap());
                let cache =
                    Cache::prepare_shared(&mut cache, source.as_ref(), 0, 0x1000).unwrap();
                let mut request = Some(Job::Templates(TemplatesRequest::take(
                    &cache, 0, 0x1000, &mut captured, hour, false,
                )));
                let mut batch = Batch::new(&pool);
                // SAFETY: the synthetic world lives until the batch drains.
                let ticket = unsafe { batch.submit(&mut request) }.unwrap();
                let completed = batch.finish(ticket).unwrap();
                assert!(matches!(completed.report, Ok(Report::Templates(_))));
                let Job::Templates(request) = completed.request else {
                    panic!("template request must be returned");
                };
                request.restore(&mut captured, None);
                // Taking the worker's publication and settling its owned tail
                // cannot read live memory, including on the promoted claim tier.
                captured.capture(
                    &SliceMemory {
                        base: 0,
                        bytes: &[],
                    },
                    0,
                    0x1000,
                    hour,
                    false,
                );
                assert_eq!(
                    captured.outcomes(),
                    direct.outcomes(),
                    "{countries}/{fault}/{hour}"
                );
                assert_eq!(
                    captured.tables(),
                    direct.tables(),
                    "{countries}/{fault}/{hour}"
                );
                assert_eq!(captured.country_state(), direct.country_state());
                assert_eq!(captured.model_failures(), direct.model_failures());
                if fault == 2 {
                    assert!(matches!(
                        captured.outcomes()[0],
                        Outcome::Unavailable { .. }
                    ));
                } else {
                    assert_eq!(
                        template_outcomes(&captured),
                        [Outcome::Rows { len: countries }; 2]
                    );
                }
            }
        }
    }
}

#[test]
fn live_template_demand_and_same_capture_growth_keep_every_country() {
    let source = large_content(20, 4);
    let mut capacity = Capacities::default();
    capacity.presize(&source, 0, 0x1000).unwrap();
    assert!(capacity.rows >= 80 && capacity.icon_rows >= 80);
    assert!(capacity.bytes >= 80 * 11);
    assert!(capacity.icon_bytes >= 80 * 20);
    assert!(capacity.counts >= 80);
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&source, 0, 0x1000, 9, true);
    assert_eq!(template_outcomes(&captured), [Outcome::Rows { len: 80 }; 2]);
    let grown = large_content(20, 5);
    captured.capture(&grown, 0, 0x1000, 10, true);
    assert_eq!(template_outcomes(&captured), [Outcome::Rows { len: 100 }; 2]);
    assert_eq!(captured.name_bytes.len, 100 * (3 + 8));
    assert_eq!(captured.counts.len, 100);
}

#[test]
fn optional_model_failure_does_not_remove_fresh_name_inputs() {
    let mut capture = Captured::with_capacities(&capacity());
    let mut source = fixture();
    source.bytes[0x5000 + native::SUBUNIT_COUNTS + 8..0x5000 + native::SUBUNIT_COUNTS + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    source.bytes[0x5000 + native::SUBUNIT_COUNTS + 12..0x5000 + native::SUBUNIT_COUNTS + 16]
        .copy_from_slice(&2_u32.to_le_bytes()); // length beyond capacity
    capture.capture(&source, 0, 0x1000, 9, true);
    let row = displayed(capture.table_inputs());
    assert_eq!(row.model_unreadable, 1);
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
}

struct CoreChanges {
    templates: crate::template_changes::Watches,
    locals: crate::local_subunit_changes::Watches,
}
impl CoreChanges {
    fn new() -> Self {
        Self {
            templates: crate::template_changes::Watches::enabled(64),
            locals: crate::local_subunit_changes::Watches::enabled(16),
        }
    }
}
impl crate::template_changes::Changes for CoreChanges {
    fn quiet(&self) -> Option<(u64, u64)> {
        crate::template_changes::Changes::quiet(&self.templates)
    }
    fn sequence(&self, pointer: usize) -> Option<(u64, u64)> {
        crate::template_changes::Changes::sequence(&self.templates, pointer)
    }
    fn core_dependencies(&self) -> Option<(u64, u64)> {
        crate::local_subunit_changes::Changes::quiet(&self.locals)
    }
}

/// Every read a capture makes through it.
struct Recording<'a> {
    source: &'a Fixture,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}
impl Memory for Recording<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, out.len()));
        self.source.copy(at, out)
    }
}
impl<'a> Recording<'a> {
    fn new(source: &'a Fixture) -> Self {
        Self {
            source,
            reads: Default::default(),
        }
    }
    /// Reads of the template object past its identity.
    fn template(&self, template: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, len)| {
                (template..template + 0x400).contains(&at) && (at, len) != (template, 16)
            })
            .count()
    }
    /// Reads of the grouped entries, grid cells and sub-unit the templates name.
    fn shared(&self) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| (0x6000..0x6400).contains(&at) || (0x7000..0x7400).contains(&at))
            .count()
    }
}

/// A copy of the fixture's template at 0x7800, listed after it.
fn add_second_template(source: &mut Fixture) {
    let template = source.bytes[0x5000..0x5400].to_vec();
    source.bytes[0x7800..0x7c00].copy_from_slice(&template);
    source.bytes[0x7808..0x7810].copy_from_slice(&((17_u64 << 32) | 52).to_le_bytes());
    source.bytes[0x4008..0x4010].copy_from_slice(&0x7800_u64.to_le_bytes());
    source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + 8..][..8]
        .copy_from_slice(&0x0000_0002_0000_0002_u64.to_le_bytes());
}

fn site(name: &str) -> usize {
    engine_mutations::template_mutations::MUTATIONS
        .iter()
        .position(|(site, _)| site.name == name)
        .unwrap()
}

/// The fixture with a regiment grid on its template at 0x5000.
fn gridded_fixture() -> Fixture {
    const A: usize = 0x5000;
    let mut source = fixture();
    source.bytes[A + native::REGIMENT_DATA..][..8].copy_from_slice(&0x6100_u64.to_le_bytes());
    source.bytes[A + native::REGIMENT_DATA + 8..][..8]
        .copy_from_slice(&0x0000_0003_0000_0003_u64.to_le_bytes());
    source.bytes[0x6100..0x6108].copy_from_slice(&0x7000_u64.to_le_bytes());
    source.bytes[0x6110..0x6118].copy_from_slice(&0x7000_u64.to_le_bytes());
    source.bytes[A + native::GRID_DIMENSIONS..][..4]
        .copy_from_slice(&0x0003_0001_u32.to_le_bytes());
    source
}

#[test]
fn held_templates_read_nothing_and_every_other_template_is_read_in_full() {
    const A: usize = 0x5000;
    const B: usize = 0x7800;
    let changes = CoreChanges::new();
    let mut source = gridded_fixture();
    let mut capture = Captured::with_capacities(&capacity());
    let mut previous = None;
    for hour in 0..16 {
        // The templates this hour reads in full.
        let read: &[usize] = match hour {
            0 => &[A],
            2 => {
                source.bytes[0x6008..0x600c].copy_from_slice(&7_i32.to_le_bytes());
                drop(changes.templates.notify(A, site("rebuild_template_caches")));
                &[A]
            }
            3 => {
                source.bytes[0x7000 + subunit::COMBAT_WIDTH..][..8]
                    .copy_from_slice(&34000_i64.to_le_bytes());
                drop(changes.locals.notify(Some(0x8400), 1));
                &[A]
            }
            4 => {
                // Derived-stat rebuilds write the formation count.
                source.bytes[A + native::FORMATION_DIVISION_COUNT..][..4]
                    .copy_from_slice(&19_i32.to_le_bytes());
                drop(changes.templates.notify(A, site("rebuild_template_caches")));
                &[A]
            }
            5 => {
                add_second_template(&mut source);
                &[B]
            }
            6 => {
                // A roster move renumbers both without reading either.
                source.bytes[0x4000..0x4008].copy_from_slice(&(B as u64).to_le_bytes());
                source.bytes[0x4008..0x4010].copy_from_slice(&(A as u64).to_le_bytes());
                &[]
            }
            7 => {
                source.bytes[B + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&0xf000_u64.to_le_bytes());
                drop(changes.templates.notify(B, site("rebuild_template_caches")));
                &[B]
            }
            // A refused row is not held.
            8 => &[B],
            9 => {
                source.bytes[B + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&0x6000_u64.to_le_bytes());
                drop(changes.templates.notify(B, site("rebuild_template_caches")));
                &[B]
            }
            10 => {
                source.bytes[A + native::NAME] = b'B';
                drop(changes.templates.notify(A, site("assign_editable_template_data")));
                &[A]
            }
            11 => {
                source.bytes[B + native::NAME_KEY..][..8].copy_from_slice(b"new_name");
                drop(changes.templates.notify(B, site("read_template_data")));
                &[B]
            }
            12 => {
                // Another handle at the held object is read, announced or not.
                source.bytes[B + 8..B + 16].copy_from_slice(&((18_u64 << 32) | 52).to_le_bytes());
                &[B]
            }
            13 => {
                source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + 8..][..8]
                    .copy_from_slice(&0x0000_0001_0000_0001_u64.to_le_bytes());
                &[]
            }
            14 => {
                source.bytes[B + native::CUSTOM_ICON..][..4].copy_from_slice(&17_i32.to_le_bytes());
                drop(crate::template_changes::native_notification(
                    &changes.templates,
                    site("set_template_symbol"),
                    0xc0de,
                    0,
                ));
                &[B]
            }
            _ => &[],
        };
        let recording = Recording::new(&source);
        capture.capture_original(&recording, 0, 0x1000, hour, false, &changes);
        let mut fresh = Captured::with_capacities(&capacity());
        fresh.capture_original(&source, 0, 0x1000, hour, true, &changes);
        assert_eq!(capture.tables(), fresh.tables(), "hour {hour}");
        assert_eq!(
            capture.model_failures(),
            fresh.model_failures(),
            "hour {hour}"
        );
        for template in [A, B] {
            assert_eq!(
                recording.template(template) > 0,
                read.contains(&template),
                "hour {hour}, template {template:#x}"
            );
        }
        if read.is_empty() {
            assert_eq!(recording.shared(), 0, "hour {hour}");
        }
        if hour == 6 {
            assert!(capture.witness.is_none(), "hour {hour}");
        }
        // An hour witnessing the previous hour's rows publishes exactly them.
        let tables = capture.tables();
        if let Some((witness, rows)) = &previous
            && capture.witness.is_some()
            && capture.witness == *witness
        {
            assert_eq!(&tables, rows, "hour {hour}");
        }
        previous = Some((capture.witness, tables));
    }
}

#[test]
fn a_writer_that_changes_no_template_keeps_the_witness_and_one_that_does_renews_it() {
    const A: usize = 0x5000;
    let changes = CoreChanges::new();
    let mut source = gridded_fixture();
    let mut capture = Captured::with_capacities(&capacity());
    let mut witnesses = Vec::new();
    for hour in 0..5 {
        match hour {
            2 => drop(changes.templates.notify(A, site("rebuild_template_caches"))),
            3 => {
                source.bytes[A + native::FORMATION_DIVISION_COUNT..][..4]
                    .copy_from_slice(&19_i32.to_le_bytes());
                drop(changes.templates.notify(A, site("rebuild_template_caches")));
            }
            _ => {}
        }
        let recording = Recording::new(&source);
        capture.capture_original(&recording, 0, 0x1000, hour, false, &changes);
        assert_eq!(recording.template(A) > 0, matches!(hour, 0 | 2 | 3), "hour {hour}");
        // The first hour reads its countries afresh.
        assert_eq!(capture.witness.is_some(), hour > 0, "hour {hour}");
        witnesses.push(capture.witness);
    }
    // Re-read unchanged after the writer.
    assert_eq!(witnesses[1], witnesses[2]);
    assert_ne!(witnesses[2], witnesses[3]);
    assert_eq!(witnesses[3], witnesses[4]);
}

#[test]
fn country_refusal_recovery_and_loss_renew_the_template_witness() {
    use engine_occupation::ccountry_occupation_status as occupation;
    use replay_data::rows::TableRows;
    let changes = CoreChanges::new();
    let mut source = gridded_fixture();
    let mut capture = Captured::with_capacities(&capacity());
    let mut witnesses = Vec::new();
    for hour in 0..7 {
        if matches!(hour, 2 | 5) {
            let pointer = if hour == 2 { 0x8000_u64 } else { 0 };
            for (at, value) in [
                (0x3000 + ccountry::OCCUPATION_STATUS, pointer),
                (0x8000, occupation::VTABLE_RVA.0 as u64),
                (
                    0x8000 + occupation::SECONDARY_BASE,
                    occupation::SECONDARY_VTABLE_RVA.0 as u64,
                ),
            ] {
                source.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
            }
            drop(changes.templates.notify(1, site("rebuild_template_caches")));
        }
        capture.capture_original(&source, 0, 0x1000, hour, false, &changes);
        let known = (2..5).contains(&hour);
        assert_eq!(capture.model_failures.rows, u32::from(!known), "hour {hour}");
        let TableView::Complete(TableRows::CountryTemplateState(rows)) = capture.country_state()
        else {
            panic!("country configuration unavailable at hour {hour}");
        };
        assert_eq!(rows[0].garrison_priority, Recorded::Value(known.then_some(0)));
        assert_eq!(matches!(rows[0].garrisons, Recorded::Value(Some(_))), known);
        assert_eq!(capture.witness.is_some(), matches!(hour, 1 | 3 | 4 | 6));
        witnesses.push(capture.witness);
    }
    assert_ne!(witnesses[1], witnesses[3]);
    assert_eq!(witnesses[3], witnesses[4]);
    assert_ne!(witnesses[4], witnesses[6]);
}

/// Fresh-only observation reads every template under standing certificates.
#[test]
fn fresh_only_observation_reads_every_template() {
    use crate::template_changes::Changes;
    struct FreshOnly(CoreChanges);
    impl Changes for FreshOnly {
        fn fresh_only(&self) -> bool {
            true
        }
        fn quiet(&self) -> Option<(u64, u64)> {
            self.0.quiet()
        }
        fn sequence(&self, pointer: usize) -> Option<(u64, u64)> {
            self.0.sequence(pointer)
        }
        fn core_dependencies(&self) -> Option<(u64, u64)> {
            self.0.core_dependencies()
        }
    }
    let changes = FreshOnly(CoreChanges::new());
    let mut source = fixture();
    let mut capture = Captured::with_capacities(&capacity());
    for hour in 0_i32..3 {
        // No writer announces the edit.
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4].copy_from_slice(&hour.to_le_bytes());
        let recording = Recording::new(&source);
        capture.capture_original(&recording, 0, 0x1000, hour, false, &changes);
        assert!(recording.template(0x5000) > 0, "hour {hour}");
        assert_eq!(displayed(capture.table_inputs()).custom_icon, hour);
    }
}

#[test]
fn tracked_variant_replacement_with_an_unchanged_header_invalidates_template_core() {
    use engine_mutations::template_mutations::{MUTATIONS, Receiver};
    use engine_native::cequipment_variant as variant;

    let site = MUTATIONS
        .iter()
        .position(|(site, _)| site.name == "update_template_tracked_variant");
    let changes = CoreChanges::new();
    let mut source = fixture();
    let pointers = [0x7500_usize, 0x7700, 0x7900];
    let handles = [21_u64, 22, 23].map(|id| (id << 32) | u64::from(variant::REFERENCE_TYPE));
    for (&pointer, handle) in pointers.iter().zip(handles) {
        source.bytes[pointer..][..8].copy_from_slice(&(variant::VTABLE_RVA.0 as u64).to_le_bytes());
        source.bytes[pointer + 8..][..8].copy_from_slice(&handle.to_le_bytes());
    }
    let header = 0x5000 + native::EXCLUDED_VARIANT_DATA;
    source.bytes[header..][..8].copy_from_slice(&0x6200_u64.to_le_bytes());
    source.bytes[header + 8..][..8].copy_from_slice(&((2_u64 << 32) | 2).to_le_bytes());
    for (index, pointer) in [pointers[1], pointers[2]].into_iter().enumerate() {
        source.bytes[0x6200 + index * 8..][..8].copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    let mut captured = Captured::with_capacities(&capacity());
    captured.capture_original(&source, 0, 0x1000, 1, false, &changes);
    assert_eq!(
        displayed(captured.table_inputs()).excluded_variants,
        handles[1..]
    );
    let fixed = source.bytes[0x5000..0x5400].to_vec();
    // Two role edits compact then append in spare storage, restoring the complete header.
    {
        let _writer = site.map(|site| {
            assert!(matches!(MUTATIONS[site].1, Receiver::Data));
            changes.templates.notify(0x5000, site)
        });
        source.bytes[0x6200..][..8].copy_from_slice(&(pointers[2] as u64).to_le_bytes());
        source.bytes[header + 12..][..4].copy_from_slice(&1_u32.to_le_bytes());
    }
    {
        let _writer = site.map(|site| changes.templates.notify(0x5000, site));
        source.bytes[0x6208..][..8].copy_from_slice(&(pointers[0] as u64).to_le_bytes());
        source.bytes[header + 12..][..4].copy_from_slice(&2_u32.to_le_bytes());
    }
    assert_eq!(&source.bytes[0x5000..0x5400], fixed.as_slice());
    captured.capture_original(&source, 0, 0x1000, 2, false, &changes);
    let mut fresh = Captured::with_capacities(&capacity());
    fresh.capture_original(&source, 0, 0x1000, 2, true, &changes);
    assert_eq!(captured.tables(), fresh.tables());
    assert_eq!(
        displayed(captured.table_inputs()).excluded_variants,
        [handles[2], handles[0]]
    );
    // A newly unreadable variant must refuse current model inputs, never carry old handles.
    {
        let _writer = site.map(|site| changes.templates.notify(0x5000, site));
        source.bytes[0x6208..][..8].copy_from_slice(&0xf000_u64.to_le_bytes());
    }
    captured.capture_original(&source, 0, 0x1000, 3, false, &changes);
    fresh.capture_original(&source, 0, 0x1000, 3, true, &changes);
    assert_eq!(captured.tables(), fresh.tables());
    let refused = displayed(captured.table_inputs());
    assert_eq!(refused.model_unreadable, 1);
    assert!(refused.excluded_variants.is_empty());
}

/// The custom-icon command names no template, so every template is read again.
#[test]
fn the_custom_icon_command_reads_every_template_again() {
    let changes = CoreChanges::new();
    let mut source = fixture();
    add_second_template(&mut source);
    let mut captured = Captured::with_capacities(&capacity());
    captured.capture_original(&source, 0, 0x1000, 1, false, &changes);
    let recording = Recording::new(&source);
    captured.capture_original(&recording, 0, 0x1000, 2, false, &changes);
    assert_eq!(recording.template(0x5000) + recording.template(0x7800), 0);
    source.bytes[0x5000 + native::CUSTOM_ICON..][..4].copy_from_slice(&17_i32.to_le_bytes());
    source.bytes[0x5000 + native::CUSTOM_ICON_NATO] = 1;
    drop(crate::template_changes::native_notification(
        &changes.templates,
        site("set_template_symbol"),
        0xc0de,
        0,
    ));
    let recording = Recording::new(&source);
    captured.capture_original(&recording, 0, 0x1000, 3, false, &changes);
    assert!(recording.template(0x5000) > 0 && recording.template(0x7800) > 0);
    let mut fresh = Captured::with_capacities(&capacity());
    fresh.capture_original(&source, 0, 0x1000, 3, true, &changes);
    assert_eq!(captured.tables(), fresh.tables());
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = captured.tables()
    else {
        panic!("icons");
    };
    let row = icons
        .iter()
        .find(|row| row.template == 0xffff_ffff_0000_0034)
        .unwrap();
    assert_eq!((row.custom_icon, row.custom_icon_nato), (17, 1));
}

#[test]
fn missing_local_certificate_withdraws_a_held_core_without_withholding_fresh_model_values() {
    let changes = CoreChanges::new();
    let mut source = fixture();
    let mut capture = Captured::with_capacities(&capacity());
    capture.capture_original(&source, 0, 0x1000, 0, false, &changes);
    source.bytes[0x6008..0x600c].copy_from_slice(&9_i32.to_le_bytes());
    let running = changes.locals.notify(None, 4);
    capture.capture_original(&source, 0, 0x1000, 1, false, &changes);
    let mut fresh = Captured::with_capacities(&capacity());
    fresh.capture_original(&source, 0, 0x1000, 1, true, &changes);
    assert_eq!(capture.tables(), fresh.tables());
    assert!(
        !changes.templates.poisoned(),
        "missing certification does not poison"
    );
    drop(running);
}

#[test]
#[ignore = "synthetic quiet template core acquisition timing"]
fn certified_template_core_scale_bench() {
    use std::time::Instant;
    const TEMPLATES: usize = 1500;
    let changes = CoreChanges {
        templates: crate::template_changes::Watches::enabled(TEMPLATES + 16),
        locals: crate::local_subunit_changes::Watches::enabled(16),
    };
    let mut source = fixture();
    for (grid, data, count) in [
        (native::REGIMENT_DATA, 0x6100, 25_u32),
        (native::SUPPORT_DATA, 0x6200, 5),
        (native::REGIMENTAL_SUPPORT_DATA, 0x6300, 5),
    ] {
        source.bytes[0x5000 + grid..][..8].copy_from_slice(&(data as u64).to_le_bytes());
        source.bytes[0x5000 + grid + 8..][..8]
            .copy_from_slice(&((u64::from(count) << 32) | u64::from(count)).to_le_bytes());
        for index in 0..count as usize {
            source.bytes[data + index * 8..][..8].copy_from_slice(&0x7000_u64.to_le_bytes());
        }
    }
    let template = source.bytes[0x5000..0x5400].to_vec();
    let list = 0xa000;
    let objects = (list + TEMPLATES * 8).next_multiple_of(0x1000);
    source.bytes.resize(objects + TEMPLATES * 0x400, 0);
    source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES..][..8]
        .copy_from_slice(&(list as u64).to_le_bytes());
    for offset in [8, 12] {
        source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + offset..][..4]
            .copy_from_slice(&(TEMPLATES as u32).to_le_bytes());
    }
    for index in 0..TEMPLATES {
        let pointer = objects + index * 0x400;
        source.bytes[pointer..pointer + 0x400].copy_from_slice(&template);
        source.bytes[pointer + 8..pointer + 16]
            .copy_from_slice(&(((index as u64 + 1) << 32) | 52).to_le_bytes());
        source.bytes[list + index * 8..][..8].copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    let capacities = Capacities {
        rows: TEMPLATES,
        icon_rows: TEMPLATES,
        counts: TEMPLATES * 16,
        bytes: TEMPLATES * 32,
        icon_bytes: TEMPLATES * 32,
        name_bytes: TEMPLATES * 32,
        ..capacity()
    };
    let mut expected = None;
    for (name, observer) in [
        (
            "fresh_core",
            &changes.templates as &dyn crate::template_changes::Changes,
        ),
        ("held_core", &changes),
    ] {
        let mut capture = Captured::with_capacities(&capacities);
        capture.capture_original(&source, 0, 0x1000, 0, false, observer);
        let mut elapsed = Vec::new();
        for hour in 1..31 {
            let start = Instant::now();
            capture.capture_original(&source, 0, 0x1000, hour, false, observer);
            elapsed.push(start.elapsed());
        }
        elapsed.sort_unstable();
        let tables = capture.tables();
        if let Some(expected) = &expected {
            assert_eq!(&tables, expected);
        } else {
            expected = Some(tables);
        }
        println!(
            "TEMPLATE_CORE_BENCH templates={TEMPLATES} mode={name} p50_us={}",
            elapsed[elapsed.len() / 2].as_micros()
        );
    }
}

#[test]
fn pooled_frames_carry_and_witness_the_previous_hours_templates_from_their_shared_home() {
    const A: usize = 0x5000;
    let changes = CoreChanges::new();
    let source = gridded_fixture();
    let mut frames = [
        Captured::with_capacities(&capacity()),
        Captured::with_capacities(&capacity()),
    ];
    let home = frames[0].home().unwrap();
    frames[1].share_home(&home);
    let mut witnesses = Vec::new();
    let mut tables = Vec::new();
    for hour in 0..4 {
        let frame = &mut frames[hour as usize % 2];
        let recording = Recording::new(&source);
        frame.capture_original(&recording, 0, 0x1000, hour, false, &changes);
        assert_eq!(recording.template(A) > 0, hour == 0, "hour {hour}");
        witnesses.push(frame.witness);
        tables.push(frame.tables());
    }
    assert!(witnesses[1].is_some());
    assert_eq!(witnesses[1], witnesses[2]);
    assert_eq!(witnesses[2], witnesses[3]);
    assert_eq!(tables[1], tables[2]);
    assert_eq!(tables[2], tables[3]);
    // A capture while the stores are out (a settlement still pending) reads afresh.
    let bytes = super::home_allocated_bytes(&home);
    assert!(bytes > 0);
    drop(home.lock().unwrap().stores.take());
    assert_eq!(super::home_allocated_bytes(&home), bytes);
    let recording = Recording::new(&source);
    frames[0].capture_original(&recording, 0, 0x1000, 4, false, &changes);
    assert!(recording.template(A) > 0);
    assert_ne!(frames[0].witness, witnesses[3]);
    assert_eq!(frames[0].tables(), tables[3]);
}
