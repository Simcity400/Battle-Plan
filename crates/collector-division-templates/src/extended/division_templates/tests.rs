use super::*;
#[cfg(test)]
use engine_native_memory::memory::CountedMemory;
use engine_native::{ccountry, cgame_state};
use engine_native_parts::csub_unit_definition as subunit;
use engine_native_memory::memory::SliceMemory;

pub struct Fixture {
    pub bytes: Vec<u8>,
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
pub fn fixture() -> Fixture {
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
pub fn template_outcomes(captured: &Captured) -> [Outcome; 2] {
    let [dictionary, icons, _] = captured.outcomes();
    [dictionary, icons]
}

pub fn capacity() -> Capacities {
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
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { synchronous.capture_original(&source, 0, 0x1000, hour, false, &changes) };
        deferred.capture_observed(&source, 0, 0x1000, hour, false, &changes, None);
        drop(changes.templates.notify(0x5000, 1));
        source.bytes[0x6008..0x600c].copy_from_slice(&(hour + 7).to_le_bytes());
        deferred.settle_owned();
        assert_eq!(deferred.tables(), synchronous.tables());
        assert_eq!(deferred.country_state(), synchronous.country_state());
        assert_eq!(deferred.model_failures(), synchronous.model_failures());
        deferred.settle_owned();
        assert_eq!(deferred.tables(), synchronous.tables());
    }
    deferred.capture_observed(&source, 0, 0x1000, 3, false, &changes, None);
    deferred.reset();
    assert!(!deferred.pending_owned);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { deferred.capture_original(&source, 0, 0x1000, 4, false, &changes) };
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { synchronous.capture_original(&source, 0, 0x1000, 4, true, &changes) };
    assert_eq!(deferred.tables(), synchronous.tables());
}

#[test]
fn enabled_unsized_startup_records_known_template_and_country_values_through_spool() {
    use crate::template_changes::Changes;
    use engine_deployment::cdeployment_status as deployment;
    use engine_occupation::ccountry_occupation_status as occupation;
    use replay_capture_spool::spool::{Record, encode_record};
    use replay_frame::{
        frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput, frame::Metrics,
    };
    const TEST: &str = "extended::division_templates::tests::enabled_unsized_startup_records_known_template_and_country_values_through_spool";
    const CHILD: &str = "BATTLEPLAN_TEMPLATE_UNSIZED_STARTUP_TEST";
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
    let reset = collector_model_inputs::local_subunit_changes::unsized_observer_for_tests();
    assert!(changes.quiet().is_some());
    assert!(
        changes.sequence(0x5000).is_none(),
        "no template watch table yet"
    );
    assert!(collector_model_inputs::local_subunit_changes::Changes::quiet(&reset).is_some());
    assert!(
        collector_model_inputs::local_subunit_changes::Changes::sequence(&reset, 0x8400).is_none(),
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

    let mut capacities = Capacities::default();
    // SAFETY: synthetic memory fixture with no live template observer hooks.
    unsafe { capacities.presize(&source, 0, 0x1000) }.unwrap();
    collector_model_inputs::local_subunit_changes::boundary(true);
    crate::template_changes::boundary(true);
    let mut capture = Captured::with_capacities(&capacities);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 1, true, &changes) };
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
fn cold_and_post_gap_template_columns_pass_spool_validation_with_named_refusals() {
    use crate::template_changes::Watches;
    use replay_capture_spool::spool::{Record, encode_record};
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
                // SAFETY: Unit test runs against an isolated fixture.
                unsafe { capture.capture_original(&source, 0, 0x1000, 0, true, &watches) };
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
            // SAFETY: Unit test runs against an isolated fixture.
            unsafe { capture.capture_original(&source, 0, 0x1000, 2, true, changes) };
            let decoded = admit(&capture, &mut encoder, 2);
            let icons = decoded.complete::<tables::DivisionTemplateIcons>().unwrap();
            if case == "empty roster" {
                assert!(icons.is_empty());
            } else {
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
            // SAFETY: Unit test runs against an isolated fixture.
            unsafe { capture.capture_original(&source, 0, 0x1000, 3, false, &watches) };
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
    let country = engine_native_memory::country_identity::Country {
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
        collector_test_support::with_allocations_denied(|| {
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
    drop(watches.notify(0x8400, 34));
    store.begin();
    store.read(&source, 0, 0x1000, country, &watches).unwrap();
    store.finish();
    assert!(!store.changed);
    drop(watches.notify(0x8000, 36));
    store.begin();
    collector_test_support::with_allocations_denied(|| {
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
    store.begin();
    store.finish();
    assert!(store.changed);
    store.begin();
    store
        .read(&Unreadable, 0, 0x1000, country, &watches)
        .unwrap();
    store.finish();
    assert!(store.changed);
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
        let reads = collector_memory::read_accounting::Capture::new();
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture(&present, 0, 0x1000, hour, force) };
        let read = reads.finish();
        assert!(read[TABLES[1].index()].spans > 0);
        assert_eq!(read[TABLES[0].index()].spans, 0, "{hour}");
        assert_eq!(
            template_outcomes(&capture),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture(&missing, 0, 0x1000, hour, force) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 1, true, &watches) };
    assert!(source.edited.get());
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::DivisionTemplateIcons(icons)),
    ] = capture.tables()
    else {
        panic!("icons");
    };
    assert!(matches!(icons[0].settings, Recorded::Value(None)));
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 2, false, &watches) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 3, true, &watches) };
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

pub fn displayed(inputs: [TableInput; N]) -> DivisionTemplateIconRow {
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
pub fn set_name(source: &mut Fixture, literal: &[u8], key: &[u8]) {
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 9, true) };
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            value: TemplateNameValue::SameAsDivisionTemplates,
            ..
        }))
    ));
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 10, false) };
    assert!(matches!(
        displayed(capture.table_inputs()).display_name,
        Recorded::Value(Some(TemplateDisplayName {
            sample_hour: replay_data::rows::PacketHour::Host,
            value: TemplateNameValue::SameAsDivisionTemplates
        }))
    ));
    set_name(&mut source, b"Renamed", b"unused");
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 11, false) };
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

#[test]
fn every_hour_certifies_against_the_same_frame_and_a_failed_dictionary_overrides() {
    let mut capture = Captured::with_capacities(&capacity());
    let source = fixture();
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 10, false) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 9, true) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&fixture(), 0, 0x1000, 9, true) };
    let row = displayed(capture.table_inputs());
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
    assert_eq!(row.priority_unit_name, b"infantry");
    assert_eq!(row.model_unreadable, 0);
    assert_eq!(capture.name_bytes.required, 3 + 8);
    capture.grow(&mut size);
    assert!(size.name_bytes >= 3 + 8);
}

pub fn large_content(countries: usize, per_country: usize) -> Fixture {
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

#[cfg(test)]
fn deployed_content(countries: usize, per_country: usize) -> Fixture {
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: usize) {
        word(bytes, at, data as u64);
        word(bytes, at + 8, (count as u64) | ((count as u64) << 32));
    }
    let mut source = fixture();
    let template = source.bytes[0x5000..0x5400].to_vec();
    let slots = countries + 1;
    let registry = 0x10000;
    let links = registry + slots * 8;
    let owners = (links + slots * 4).next_multiple_of(0x1000);
    let statuses = owners + countries * 0x1000;
    let rosters = statuses + countries * 0x200;
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
        let owner = owners + country * 0x1000;
        let status = statuses + country * 0x200;
        word(&mut source.bytes, registry + id * 8, owner as u64);
        source.bytes[links + id * 4..links + id * 4 + 4]
            .copy_from_slice(&(id as u32).to_le_bytes());
        word(&mut source.bytes, owner, ccountry::VTABLE_RVA.0 as u64);
        word(&mut source.bytes, owner + ccountry::ID, id as u64);
        word(
            &mut source.bytes,
            owner + ccountry::DEPLOYMENT_MANAGER,
            status as u64,
        );
        word(
            &mut source.bytes,
            status,
            engine_deployment::cdeployment_status::VTABLE_RVA.0 as u64,
        );
        word(
            &mut source.bytes,
            status + engine_deployment::cdeployment_status::OWNER,
            owner as u64,
        );
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
fn split_walk_equals_the_serial_walk_and_every_range_walks_its_share() {
    const COUNTRIES: usize = 40;
    const PER_COUNTRY: usize = 24;
    const TOTAL: usize = COUNTRIES * PER_COUNTRY;
    fn word(source: &Fixture, at: usize) -> usize {
        u64::from_le_bytes(source.bytes[at..at + 8].try_into().unwrap()) as usize
    }
    fn set(source: &mut Fixture, at: usize, value: usize) {
        source.bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
    let entry = |source: &Fixture, country: usize, ordinal: usize| {
        let owner = word(source, 0x10000 + (country + 1) * 8);
        let at = word(source, owner + ccountry::DIVISION_TEMPLATES) + ordinal * 8;
        (at, word(source, at))
    };
    for lanes in [1, 2, 3, 7, 15] {
        let mut source = deployed_content(COUNTRIES, PER_COUNTRY);
        let changes = CoreChanges::sized(4096, 256);
        let mut direct = Captured::with_capacities(&capacity());
        let mut split = Captured::with_capacities(&capacity());
        let (duplicate, listed) = entry(&source, 30, 0);
        let (refused, kept) = entry(&source, 20, 3);
        for hour in 0..7 {
            match hour {
                2 => {
                    for country in [0, 19, 39] {
                        let (_, template) = entry(&source, country, 5);
                        source.bytes[template + native::FORMATION_DIVISION_COUNT..][..4]
                            .copy_from_slice(&(hour + country as i32).to_le_bytes());
                        drop(
                            changes
                                .templates
                                .notify(template, site("rebuild_template_caches")),
                        );
                    }
                }
                3 => {
                    let (_, other) = entry(&source, 5, 0);
                    set(&mut source, duplicate, other);
                }
                5 => {
                    set(&mut source, duplicate, listed);
                    set(&mut source, refused, 0);
                }
                6 => set(&mut source, refused, kept),
                _ => {}
            }
            let force = hour == 0;
            // SAFETY: Unit test runs against an isolated fixture.
            unsafe { direct.capture_original(&source, 0, 0x1000, hour, force, &changes) };
            let mut shares = Vec::new();
            let mut walk = |mut parts: Vec<WalkPart>| {
                std::thread::scope(|scope| {
                    for part in parts.iter_mut() {
                        let (source, changes) = (&source, &changes);
                        scope.spawn(move || part.capture(source, 0, changes));
                    }
                });
                shares.extend(parts.iter().map(WalkPart::visited));
                parts.reverse();
                Ok::<_, &'static str>(parts)
            };
            split.capture_observed(
                &source,
                0,
                0x1000,
                hour,
                force,
                &changes,
                Some(TemplateWorkers {
                    lanes,
                    walk: &mut walk,
                }),
            );
            split.settle_owned();
            let label = format!("{lanes} workers, hour {hour}");
            let len = direct.icon_rows.outcome.len();
            assert_eq!(split.outcomes(), direct.outcomes(), "{label}");
            assert_eq!(
                split.icon_rows.rows[..len],
                direct.icon_rows.rows[..len],
                "{label}"
            );
            assert_eq!(split.tables(), direct.tables(), "{label}");
            assert_eq!(split.country_state(), direct.country_state(), "{label}");
            assert_eq!(split.model_failures(), direct.model_failures(), "{label}");
            assert_eq!(split.speed_spans(), direct.speed_spans(), "{label}");
            assert_eq!(split.occurrences(), direct.occurrences(), "{label}");
            use collector_workers::hour_stages::TemplateFallback;
            let fallback = (lanes == 1).then_some(TemplateFallback::OneLane);
            assert_eq!(split.split_fallback(), fallback, "{label}");
            if lanes > 1 && hour == 4 {
                assert!(split.taken_again() > 0, "{label}");
            }
            match hour {
                0 => assert_eq!(direct.occurrences(), [0, TOTAL as u64], "{label}"),
                1 => assert_eq!(direct.occurrences(), [TOTAL as u64, 0], "{label}"),
                6 => assert!(direct.occurrences()[0] > 0, "{label}"),
                5 => assert!(
                    matches!(direct.outcomes()[1], Outcome::Unavailable { .. }),
                    "{label}"
                ),
                _ => {}
            }
            if lanes == 1 {
                assert!(shares.is_empty(), "{label}");
            } else if hour == 0 {
                let parts = lanes.min(TOTAL / 64);
                assert_eq!(shares.len(), parts, "{label}");
                assert_eq!(shares.iter().sum::<usize>(), TOTAL, "{label}");
                assert!(
                    shares
                        .iter()
                        .all(|&share| share + 2 * PER_COUNTRY >= TOTAL / parts),
                    "{label}: {shares:?}"
                );
            }
        }
    }
}

#[test]
fn live_template_demand_and_same_capture_growth_keep_every_country() {
    let source = large_content(20, 4);
    let mut capacity = Capacities::default();
    // SAFETY: synthetic memory fixture with no live template observer hooks.
    unsafe { capacity.presize(&source, 0, 0x1000) }.unwrap();
    assert!(capacity.rows >= 80 && capacity.icon_rows >= 80);
    assert!(capacity.bytes >= 80 * 11);
    assert!(capacity.icon_bytes >= 80 * 20);
    assert!(capacity.counts >= 80);
    let mut captured = Captured::with_capacities(&capacity);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture(&source, 0, 0x1000, 9, true) };
    assert_eq!(template_outcomes(&captured), [Outcome::Rows { len: 80 }; 2]);
    let grown = large_content(20, 5);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture(&grown, 0, 0x1000, 10, true) };
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
        .copy_from_slice(&2_u32.to_le_bytes());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture(&source, 0, 0x1000, 9, true) };
    let row = displayed(capture.table_inputs());
    assert_eq!(row.model_unreadable, 1);
    assert!(matches!(row.display_name, Recorded::Value(Some(_))));
}

#[cfg(test)]
struct CoreChanges {
    templates: crate::template_changes::Watches,
    locals: collector_model_inputs::local_subunit_changes::Watches,
}
#[cfg(test)]
impl CoreChanges {
    #[cfg(test)]
    fn new() -> Self {
        Self::sized(64, 16)
    }

    #[cfg(test)]
    fn sized(templates: usize, locals: usize) -> Self {
        Self {
            templates: crate::template_changes::Watches::enabled(templates),
            locals: collector_model_inputs::local_subunit_changes::Watches::enabled(locals),
        }
    }
}
#[cfg(test)]
impl crate::template_changes::Changes for CoreChanges {
    fn quiet(&self) -> Option<(u64, u64)> {
        crate::template_changes::Changes::quiet(&self.templates)
    }
    fn sequence(&self, pointer: usize) -> Option<(u64, u64)> {
        crate::template_changes::Changes::sequence(&self.templates, pointer)
    }
    fn core_dependencies(&self) -> Option<(u64, u64)> {
        collector_model_inputs::local_subunit_changes::Changes::quiet(&self.locals)
    }
    fn deployment_reset_sequence(&self, status: usize) -> Option<(u64, u64)> {
        collector_model_inputs::local_subunit_changes::Changes::sequence(&self.locals, status)
    }
    fn dirty_cursor(&self) -> Option<u64> {
        crate::template_changes::Changes::dirty_cursor(&self.templates)
    }
    fn dirty_receivers_since(&self, since: u64, out: &mut Vec<usize>) -> Option<u64> {
        crate::template_changes::Changes::dirty_receivers_since(&self.templates, since, out)
    }
}

#[cfg(test)]
struct Recording<'a> {
    source: &'a Fixture,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}
#[cfg(test)]
impl Memory for Recording<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, out.len()));
        self.source.copy(at, out)
    }
}
#[cfg(test)]
impl<'a> Recording<'a> {
    #[cfg(test)]
    fn new(source: &'a Fixture) -> Self {
        Self {
            source,
            reads: Default::default(),
        }
    }
    #[cfg(test)]
    fn template(&self, template: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, len)| {
                (template..template + 0x400).contains(&at) && (at, len) != (template, 16)
            })
            .count()
    }
    #[cfg(test)]
    fn shared(&self) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| (0x6000..0x6400).contains(&at) || (0x7000..0x7400).contains(&at))
            .count()
    }
}

#[cfg(test)]
fn add_second_template(source: &mut Fixture) {
    let template = source.bytes[0x5000..0x5400].to_vec();
    source.bytes[0x7800..0x7c00].copy_from_slice(&template);
    source.bytes[0x7808..0x7810].copy_from_slice(&((17_u64 << 32) | 52).to_le_bytes());
    source.bytes[0x4008..0x4010].copy_from_slice(&0x7800_u64.to_le_bytes());
    source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + 8..][..8]
        .copy_from_slice(&0x0000_0002_0000_0002_u64.to_le_bytes());
}

#[cfg(test)]
fn site(name: &str) -> usize {
    engine_mutations::template_mutations::MUTATIONS
        .iter()
        .position(|(site, _)| site.name == name)
        .unwrap()
}

#[cfg(test)]
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
        let read: &[usize] = match hour {
            0 => &[A],
            1 => {
                drop(changes.locals.notify(Some(0x9900), 1));
                &[]
            }
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
            8 => &[B],
            9 => {
                source.bytes[B + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&0x6000_u64.to_le_bytes());
                drop(changes.templates.notify(B, site("rebuild_template_caches")));
                &[B]
            }
            10 => {
                source.bytes[A + native::NAME] = b'B';
                drop(
                    changes
                        .templates
                        .notify(A, site("assign_editable_template_data")),
                );
                &[A]
            }
            11 => {
                source.bytes[B + native::NAME_KEY..][..8].copy_from_slice(b"new_name");
                drop(changes.templates.notify(B, site("read_template_data")));
                &[B]
            }
            12 => {
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
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture_original(&recording, 0, 0x1000, hour, false, &changes) };
        let mut fresh = Captured::with_capacities(&capacity());
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { fresh.capture_original(&source, 0, 0x1000, hour, true, &changes) };
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
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture_original(&recording, 0, 0x1000, hour, false, &changes) };
        assert_eq!(recording.template(A) > 0, matches!(hour, 0 | 2 | 3), "hour {hour}");
        assert_eq!(capture.witness.is_some(), hour > 0, "hour {hour}");
        witnesses.push(capture.witness);
    }
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
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture_original(&source, 0, 0x1000, hour, false, &changes) };
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
        fn deployment_reset_sequence(&self, status: usize) -> Option<(u64, u64)> {
            self.0.deployment_reset_sequence(status)
        }
    }
    let changes = FreshOnly(CoreChanges::new());
    let mut source = fixture();
    let mut capture = Captured::with_capacities(&capacity());
    for hour in 0_i32..3 {
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4].copy_from_slice(&hour.to_le_bytes());
        let recording = Recording::new(&source);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture_original(&recording, 0, 0x1000, hour, false, &changes) };
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
        source.bytes[pointer..][..8].copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
        source.bytes[pointer + 8..][..8].copy_from_slice(&handle.to_le_bytes());
    }
    let header = 0x5000 + native::EXCLUDED_VARIANT_DATA;
    source.bytes[header..][..8].copy_from_slice(&0x6200_u64.to_le_bytes());
    source.bytes[header + 8..][..8].copy_from_slice(&((2_u64 << 32) | 2).to_le_bytes());
    for (index, pointer) in [pointers[1], pointers[2]].into_iter().enumerate() {
        source.bytes[0x6200 + index * 8..][..8].copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    let mut captured = Captured::with_capacities(&capacity());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&source, 0, 0x1000, 1, false, &changes) };
    assert_eq!(
        displayed(captured.table_inputs()).excluded_variants,
        handles[1..]
    );
    let fixed = source.bytes[0x5000..0x5400].to_vec();
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&source, 0, 0x1000, 2, false, &changes) };
    let mut fresh = Captured::with_capacities(&capacity());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { fresh.capture_original(&source, 0, 0x1000, 2, true, &changes) };
    assert_eq!(captured.tables(), fresh.tables());
    assert_eq!(
        displayed(captured.table_inputs()).excluded_variants,
        [handles[2], handles[0]]
    );
    {
        let _writer = site.map(|site| changes.templates.notify(0x5000, site));
        source.bytes[0x6208..][..8].copy_from_slice(&0xf000_u64.to_le_bytes());
    }
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&source, 0, 0x1000, 3, false, &changes) };
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { fresh.capture_original(&source, 0, 0x1000, 3, true, &changes) };
    assert_eq!(captured.tables(), fresh.tables());
    let refused = displayed(captured.table_inputs());
    assert_eq!(refused.model_unreadable, 1);
    assert!(refused.excluded_variants.is_empty());
}

#[test]
fn the_custom_icon_command_reads_every_template_again() {
    let changes = CoreChanges::new();
    let mut source = fixture();
    add_second_template(&mut source);
    let mut captured = Captured::with_capacities(&capacity());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&source, 0, 0x1000, 1, false, &changes) };
    let recording = Recording::new(&source);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&recording, 0, 0x1000, 2, false, &changes) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { captured.capture_original(&recording, 0, 0x1000, 3, false, &changes) };
    assert!(recording.template(0x5000) > 0 && recording.template(0x7800) > 0);
    let mut fresh = Captured::with_capacities(&capacity());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { fresh.capture_original(&source, 0, 0x1000, 3, true, &changes) };
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
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 0, false, &changes) };
    source.bytes[0x6008..0x600c].copy_from_slice(&9_i32.to_le_bytes());
    let running = changes.locals.notify(None, 4);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { capture.capture_original(&source, 0, 0x1000, 1, false, &changes) };
    let mut fresh = Captured::with_capacities(&capacity());
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { fresh.capture_original(&source, 0, 0x1000, 1, true, &changes) };
    assert_eq!(capture.tables(), fresh.tables());
    assert!(
        !changes.templates.poisoned(),
        "missing certification does not poison"
    );
    drop(running);
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
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { frame.capture_original(&recording, 0, 0x1000, hour, false, &changes) };
        assert_eq!(recording.template(A) > 0, hour == 0, "hour {hour}");
        witnesses.push(frame.witness);
        tables.push(frame.tables());
    }
    assert!(witnesses[1].is_some());
    assert_eq!(witnesses[1], witnesses[2]);
    assert_eq!(witnesses[2], witnesses[3]);
    assert_eq!(tables[1], tables[2]);
    assert_eq!(tables[2], tables[3]);
    let bytes = super::home_allocated_bytes(&home);
    assert!(bytes > 0);
    drop(home.lock().unwrap().stores.take());
    assert_eq!(super::home_allocated_bytes(&home), bytes);
    let recording = Recording::new(&source);
    // SAFETY: Unit test runs against an isolated fixture.
    unsafe { frames[0].capture_original(&recording, 0, 0x1000, 4, false, &changes) };
    assert!(recording.template(A) > 0);
    assert_ne!(frames[0].witness, witnesses[3]);
    assert_eq!(frames[0].tables(), tables[3]);
}

#[test]
fn clean_whole_table_fast_path_matches_full_walk_and_invalidates_on_change() {
    const A: usize = 0x5000;
    let changes = CoreChanges::new();
    let mut source = gridded_fixture();
    let mut fast = Captured::with_capacities(&capacity());
    let mut reference = Captured::with_capacities(&capacity());
    for hour in 0..3 {
        let recording = Recording::new(&source);
        // SAFETY: Isolated unit test fixture owns both captures and mock memories.
        unsafe {
            fast.capture_original(&recording, 0, 0x1000, hour, hour == 0, &changes);
            reference.capture_original(&source, 0, 0x1000, hour, true, &changes);
        }
        assert_eq!(fast.tables(), reference.tables(), "hour {hour}");
        assert_eq!(fast.country_state(), reference.country_state(), "hour {hour}");
        assert_eq!(fast.outcomes(), reference.outcomes(), "hour {hour}");
        assert_eq!(fast.model_failures(), reference.model_failures(), "hour {hour}");
        if hour == 2 {
            assert!(fast.fast_clean.is_some());
            assert_eq!(recording.template(A), 0);
            assert_eq!(recording.shared(), 0);
        }
    }
    source.bytes[A + native::FORMATION_DIVISION_COUNT..][..4]
        .copy_from_slice(&9_i32.to_le_bytes());
    drop(changes.templates.notify(A, site("rebuild_template_caches")));
    let recording = Recording::new(&source);
    // SAFETY: Isolated unit test fixture owns both captures and mock memories.
    unsafe {
        fast.capture_original(&recording, 0, 0x1000, 3, false, &changes);
        reference.capture_original(&source, 0, 0x1000, 3, true, &changes);
    }
    assert!(fast.fast_clean.is_none());
    assert!(recording.template(A) > 0);
    assert_eq!(fast.tables(), reference.tables());
    assert_eq!(fast.country_state(), reference.country_state());
}

#[test]
fn dirty_receiver_in_place_patch_reads_only_dirty_template_and_matches_full_walk() {
    const A: usize = 0x5000;
    const B: usize = 0x5400;
    let changes = CoreChanges::new();
    let mut source = gridded_fixture();
    let mut template_b = source.bytes[A..A + 0x400].to_vec();
    template_b[8..16].copy_from_slice(&0xffff_fffe_0000_0034_u64.to_le_bytes());
    source.bytes[B..B + 0x400].copy_from_slice(&template_b);
    source.bytes[0x4008..0x4010].copy_from_slice(&(B as u64).to_le_bytes());
    source.bytes[0x3000 + ccountry::DIVISION_TEMPLATES + 8..][..8]
        .copy_from_slice(&(2_u64 | (2_u64 << 32)).to_le_bytes());
    let mut cap = capacity();
    cap.rows = 4;
    cap.icon_rows = 4;
    cap.bytes = 128;
    cap.icon_bytes = 128;
    cap.name_bytes = 128;
    cap.counts = 64;
    cap.variants = 16;
    let mut fast = Captured::with_capacities(&cap);
    let mut reference = Captured::with_capacities(&cap);
    for hour in 0..2 {
        // SAFETY: Isolated unit test fixture owns both captures and mock memories.
        unsafe {
            fast.capture_original(&source, 0, 0x1000, hour, hour == 0, &changes);
            reference.capture_original(&source, 0, 0x1000, hour, true, &changes);
        }
        assert_eq!(fast.tables(), reference.tables(), "hour {hour}");
    }
    source.bytes[A + native::FORMATION_DIVISION_COUNT..][..4]
        .copy_from_slice(&13_i32.to_le_bytes());
    drop(changes.templates.notify(A, site("rebuild_template_caches")));
    let recording = Recording::new(&source);
    // SAFETY: Isolated unit test fixture owns both captures and mock memories.
    unsafe {
        fast.capture_original(&recording, 0, 0x1000, 2, false, &changes);
        reference.capture_original(&source, 0, 0x1000, 2, true, &changes);
    }
    assert!(recording.template(A) > 0);
    assert_eq!(recording.template(B), 0);
    assert_eq!(fast.occurrences(), [1, 1]);
    assert_eq!(fast.tables(), reference.tables());
    assert_eq!(fast.country_state(), reference.country_state());
    let scratch = collector_projection::projection::ProjectionScratch::default();
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut reference_encoder = replay_frame::frame::FrameEncoder::new();
    for hour in [3, 4] {
        if hour == 4 {
            source.bytes[A + native::FORMATION_DIVISION_COUNT..][..4]
                .copy_from_slice(&17_i32.to_le_bytes());
            drop(changes.templates.notify(A, site("rebuild_template_caches")));
        }
        // SAFETY: Isolated unit test fixture owns both captures and mock memories.
        unsafe {
            fast.capture_original(&source, 0, 0x1000, hour, false, &changes);
            reference.capture_original(&source, 0, 0x1000, hour, true, &changes);
        }
        let mut projection =
            collector_projection::projection::Projection::new(&encoder, 1, hour, false, &scratch);
        let actual = fast.projected_inputs(Some(&mut projection));
        let expected = reference.table_inputs();
        let make_frame = |inputs: [TableInput; N]| {
            let mut frame = replay_frame::frame::FrameInput {
                branch: 1,
                hour,
                tables: std::array::from_fn(|_| unavailable("fixture").into()),
                metrics: replay_frame::frame::Metrics::default(),
            };
            for (table, input) in TABLES.into_iter().zip(inputs) {
                frame.tables[table.index()] = input;
            }
            frame
        };
        assert_eq!(
            encoder.encode_input(make_frame(actual), false).unwrap(),
            reference_encoder
                .encode_input(make_frame(expected), false)
                .unwrap(),
            "hour {hour}"
        );
    }
}

#[test]
fn unmatched_temporary_template_receiver_reuses_clean_state_without_rereading() {
    const A: usize = 0x5000;
    const TEMP: usize = 0x9990_0000;
    let changes = CoreChanges::new();
    let source = gridded_fixture();
    let mut fast = Captured::with_capacities(&capacity());
    let mut reference = Captured::with_capacities(&capacity());
    for hour in 0..2 {
        // SAFETY: Isolated unit test fixture owns both captures and mock memories.
        unsafe {
            fast.capture_original(&source, 0, 0x1000, hour, hour == 0, &changes);
            reference.capture_original(&source, 0, 0x1000, hour, true, &changes);
        }
    }
    let witness_before = fast.witness;
    drop(changes.templates.notify(TEMP, site("rebuild_template_caches")));
    let recording = Recording::new(&source);
    // SAFETY: Isolated unit test fixture owns both captures and mock memories.
    unsafe {
        fast.capture_original(&recording, 0, 0x1000, 2, false, &changes);
        reference.capture_original(&source, 0, 0x1000, 2, true, &changes);
    }
    assert!(fast.fast_clean.is_some());
    assert_eq!(recording.template(A), 0);
    assert_eq!(fast.occurrences(), [1, 0]);
    assert_eq!(fast.witness, witness_before);
    assert_eq!(fast.tables(), reference.tables());
    assert_eq!(fast.country_state(), reference.country_state());
}

#[test]
fn lazy_clean_materialization_refuses_allocation_failure_without_caching_none() {
    thread_local! {
        static WATCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static HOOK_FIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    fn record_failure() {
        WATCHED.with(|watched| {
            if watched.get() {
                HOOK_FIRED.with(|fired| fired.set(true));
            }
        });
    }
    struct HookGuard;
    impl Drop for HookGuard {
        fn drop(&mut self) {
            WATCHED.with(|watched| watched.set(false));
            engine_allocation::set_hooks(None, None);
        }
    }

    let changes = CoreChanges::new();
    let source = gridded_fixture();
    let mut fast = Captured::with_capacities(&capacity());
    let mut reference = Captured::with_capacities(&capacity());
    for hour in 0..3 {
        // SAFETY: Isolated unit test fixture owns both captures and mock memories.
        unsafe {
            fast.capture_original(&source, 0, 0x1000, hour, hour == 0, &changes);
            reference.capture_original(&source, 0, 0x1000, hour, true, &changes);
        }
    }
    HOOK_FIRED.with(|fired| fired.set(false));
    WATCHED.with(|watched| watched.set(true));
    engine_allocation::set_hooks(None, Some(record_failure));
    let _hook = HookGuard;

    let clean = &fast.fast_clean.as_ref().expect("clean snapshot").clean;
    let mut refusals = 0_usize;
    for ordinal in 1.. {
        let guard = collector_test_support::NullGuard::new(ordinal);
        let materialized = clean.tables();
        let denied = collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        assert!(!HOOK_FIRED.with(std::cell::Cell::get), "ordinal {ordinal}");
        if denied == 0 {
            assert!(materialized.is_some(), "ordinal {ordinal}");
            break;
        }
        assert!(materialized.is_none(), "ordinal {ordinal}");
        refusals += 1;
    }
    assert!(refusals >= 4);

    let mut publish_refusals = 0_usize;
    for ordinal in 1.. {
        clean.refill_fallback_reasons();
        let guard = collector_test_support::NullGuard::new(ordinal);
        let [dictionary, icons, _] = fast.projected_inputs(None);
        let denied = collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        assert!(
            !HOOK_FIRED.with(std::cell::Cell::get),
            "publish ordinal {ordinal}"
        );
        if denied == 0 {
            assert!(matches!(
                dictionary,
                TableInput::Sampled(TableView::Complete(_))
            ));
            assert!(matches!(icons, TableInput::Sampled(TableView::Complete(_))));
            break;
        }
        let mut saw_unavailable = false;
        for input in [&dictionary, &icons] {
            if let TableInput::Sampled(TableView::Unavailable { reason }) = input {
                assert_eq!(reason, CLEAN_ALLOC_FAILURE, "publish ordinal {ordinal}");
                saw_unavailable = true;
            }
        }
        assert!(saw_unavailable, "publish ordinal {ordinal}");
        publish_refusals += 1;
    }
    assert!(publish_refusals >= 2);
    assert_eq!(fast.tables(), reference.tables());
}
