use collector_division_templates::extended::division_templates::{
    Captured, TABLES,
    tests::{capacity, displayed, fixture, set_name},
};
use engine_native::ccountry;
use engine_native::creferenced_division_template as native;
use replay_data::rows::{Recorded, TableId, tables};
use replay_frame::frame::{TableInput, TableView};

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
        // SAFETY: Isolated template worker test owns its synthetic fixture.
        unsafe { captured.capture_original(&source, 0, 0x1000, hour, false, &Observer) };
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
    // SAFETY: Isolated template worker test owns its synthetic fixture.
    unsafe { captured.capture_original(&source, 0, 0x1000, 3, false, &Observer) };
    assert_eq!(displayed(captured.table_inputs()).model_unreadable, 1);
}
