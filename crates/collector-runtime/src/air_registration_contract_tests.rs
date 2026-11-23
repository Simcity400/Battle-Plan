use collector_air::extended::{
    air::{
        AirBasesJob, RegistrationStore, air_bases_test_kit as kit,
        bases::CarryRun,
        registration_store_tests::{
            B0_HANDLE, assert_same, captured_hour, counts, direct, fixture,
        },
    },
    fog,
};
use engine_native_air::ccountry_air_container;
use replay_data::rows::{self, TableId};
use replay_frame::frame::{TableInput, TableView};

#[test]
fn failed_registration_observer_and_forwarder_read_changed_native_subtrees() {
    use crate::hook_failures::{FamilyHooks, contract_tests};
    use collector_hook_failures::{Family, failed};
    use collector_observers::air_base_carry_changes::Changes;
    use replay_frame::frame::{FrameInput, Metrics};
    if !contract_tests::isolated(
        "air_registration_contract_tests::failed_registration_observer_and_forwarder_read_changed_native_subtrees",
    ) {
        return;
    }
    for forwarder in [false, true] {
        contract_tests::reset_session();
        if forwarder {
            FamilyHooks::<collector_ship_models::ship_model_changes::Hooks>::construct(0, || {
                Err("synthetic relay allocation failure")
            })
            .unwrap();
            assert!(failed(Family::ShipModels));
        } else {
            FamilyHooks::<collector_observers::air_base_carry_changes::Hooks>::construct(0, || {
                Err("synthetic relay allocation failure")
            })
            .unwrap();
        }
        assert!(failed(Family::AirBaseCarry));
        assert_eq!(
            collector_observers::air_base_carry_changes::Observer.sequence(B0_HANDLE),
            None
        );
        let mut bytes = fixture();
        let mut job = AirBasesJob::fixture();
        job.registration_store = Some(Box::new(RegistrationStore::new()));
        job.carry = CarryRun::default();
        let mut previous = None;
        for value in [13_i64, 29] {
            kit::put64(
                &mut bytes,
                kit::CONT1 + ccountry_air_container::CAPACITY_RAW,
                value as u64,
            );
            let (decoded, reads, work) = captured_hour(&bytes, &mut job);
            let fresh = decoded.registrations_snapshot();
            assert_eq!(work, counts(0, 2));
            assert!(
                reads
                    .iter()
                    .any(|at| (kit::CONT1..kit::CONT1 + 0x100).contains(at))
            );
            assert_same(&fresh, &direct(&bytes));
            if let Some(previous) = previous {
                assert_ne!(fresh.rows, previous);
            }
            previous = Some(fresh.rows);
            let mut frame = FrameInput {
                branch: 1,
                hour: 10,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| {
                    TableView::Complete(rows::TableRows::empty(TableId::ALL[index])).into()
                }),
            };
            for (table, view) in fog::TABLES.into_iter().zip(decoded.tables()) {
                frame.tables[table.index()] = view.into();
            }
            let expected = frame.tables[TableId::AirBaseRegistrations.index()].clone();
            assert!(
                matches!(&expected, TableInput::Sampled(TableView::Complete(rows::TableRows::AirBaseRegistrations(rows))) if !rows.is_empty())
            );
            collector_hook_failures::apply(&mut frame).unwrap();
            assert_eq!(
                frame.tables[TableId::AirBaseRegistrations.index()],
                expected
            );
        }
    }
}
