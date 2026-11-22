use collector_division_templates::extended::division_templates::{
    Captured, TABLES,
    tests::{Fixture, capacity, displayed, fixture, large_content, set_name, template_outcomes},
};
use engine_native::creferenced_division_template as native;
use engine_native::{ccountry, memory::SliceMemory, walk::Outcome};
use replay_data::rows::{Recorded, TableId, tables};
use replay_frame::frame::{TableInput, TableView};
use std::sync::Arc;

#[test]
fn failed_template_observer_preserves_fresh_template_and_country_values() {
    use crate::hook_failures::FamilyHooks;
    use collector_division_templates::template_changes::{Changes, Observer};
    use collector_hook_failures::Family;
    use engine_deployment::cdeployment_status as deployment;
    use engine_occupation::ccountry_occupation_status as occupation;
    use replay_frame::frame::{FrameInput, Metrics};

    const TEST: &str = "template_worker_tests::failed_template_observer_preserves_fresh_template_and_country_values";
    const CHILD: &str = "BATTLEPLAN_TEMPLATE_INSTALL_FAILURE_TEST";
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
    collector_primitives::build_gate::record(true, None);
    let hooks =
        FamilyHooks::<collector_division_templates::template_changes::Hooks>::construct(0, || {
            Err("synthetic nearby relay allocation failure")
        })
        .unwrap();
    assert!(hooks.inner.is_none());
    assert!(collector_hook_failures::failed(Family::Templates));
    assert!(!collector_hook_failures::failed(Family::ArmyStats));
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
        (
            0x8000 + occupation::SECONDARY_BASE,
            occupation::SECONDARY_VTABLE_RVA.0 as u64,
        ),
    ] {
        source.bytes[at..][..8].copy_from_slice(&value.to_le_bytes());
    }
    source.bytes[0x5000 + native::GRID_DIMENSIONS..][..4]
        .copy_from_slice(&((2_u32 << 16) | 1).to_le_bytes());
    let mut captured = Captured::with_capacities(&capacity());
    for (hour, key, priority) in [
        (1, b"first".as_slice(), -7_i32),
        (2, b"later".as_slice(), 11),
    ] {
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
            tables: std::array::from_fn(|index| {
                TableView::Complete(replay_data::rows::TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        for (table, value) in TABLES.into_iter().zip(captured.table_inputs()) {
            input.tables[table.index()] = value;
        }
        collector_hook_failures::apply(&mut input).unwrap();
        let TableInput::Sampled(TableView::Complete(rows)) = &input.tables[TABLES[0].index()]
        else {
            panic!("readable dictionary was discarded");
        };
        assert_eq!(
            rows.rows::<tables::DivisionTemplates>().unwrap()[0].name_key,
            Recorded::Value(key.to_vec())
        );
        let TableInput::Sampled(TableView::Complete(rows)) = &input.tables[TABLES[1].index()]
        else {
            panic!("readable template was discarded");
        };
        let row = &rows.rows::<tables::DivisionTemplateIcons>().unwrap()[0];
        let settings = row.settings.require("settings").unwrap().as_ref().unwrap();
        assert_eq!(settings.priority, priority);
        assert!(row.layout.require("layout").unwrap().is_some());
        let rows = match &input.tables[TABLES[2].index()] {
            TableInput::Sampled(TableView::Complete(rows)) => rows,
            TableInput::SharedComplete(rows) => rows.as_ref(),
            _ => panic!("readable country configuration was discarded"),
        };
        let row = &rows.rows::<tables::CountryTemplateState>().unwrap()[0];
        assert_eq!(row.next_hq_order, priority);
        assert_eq!(row.garrison_priority, Recorded::Value(Some(priority)));
        assert!(matches!(
            &input.tables[TableId::OrdersGroupCounterInputs.index()],
            TableInput::Sampled(TableView::Complete(_))
        ));
    }
    source.bytes[0x5000 + native::SUBUNIT_COUNTS..][..8].copy_from_slice(&0xf000_u64.to_le_bytes());
    captured.capture_original(&source, 0, 0x1000, 3, false, &Observer);
    assert_eq!(displayed(captured.table_inputs()).model_unreadable, 1);
}

#[test]
fn hourly_template_job_publishes_direct_walk_rows() {
    use crate::acquisition_jobs::TemplatesRequest;
    use engine_native::country_memory::Cache;

    let mut source = fixture();
    let mut capture = Captured::vacant();
    for (hour, literal, custom_icon) in [(7, &b"Alpha"[..], 11_i32), (8, &b"Bravo"[..], 22)] {
        set_name(&mut source, literal, b"fallback");
        source.bytes[0x5000 + native::CUSTOM_ICON..][..4]
            .copy_from_slice(&custom_icon.to_le_bytes());
        let mut countries =
            engine_allocation::shared(Cache::try_with_capacity(256).unwrap()).unwrap();
        let countries = Cache::prepare_shared(&mut countries, &source, 0, 0x1000).unwrap();
        let mut request = TemplatesRequest::take(&countries, 0, 0x1000, &mut capture, hour, false);
        request.run(&source);
        request.restore(&mut capture, None);
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
fn promoted_template_worker_matches_direct_rows_and_faults() {
    use crate::acquisition_jobs::{Batch, Job, Pool, Report, TemplatesRequest};
    use collector_workers::roles::Role;
    use engine_native::country_memory::Cache;

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
                1 => source.bytes[template + native::SUBUNIT_COUNTS..][..8]
                    .copy_from_slice(&u64::MAX.to_le_bytes()),
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
                let mut cache =
                    engine_allocation::shared(Cache::try_with_capacity(countries + 1).unwrap())
                        .unwrap();
                let cache = Cache::prepare_shared(&mut cache, source.as_ref(), 0, 0x1000).unwrap();
                let mut request = Some(Job::Templates(TemplatesRequest::take(
                    &cache,
                    0,
                    0x1000,
                    &mut captured,
                    hour,
                    false,
                )));
                let mut batch = Batch::new(&pool);
                let ticket = unsafe { batch.submit(&mut request) }.unwrap();
                let completed = batch.finish(ticket).unwrap();
                assert!(matches!(completed.report, Ok(Report::Templates(_))));
                let Job::Templates(request) = completed.request else {
                    panic!("template request must be returned");
                };
                request.restore(&mut captured, None);
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
