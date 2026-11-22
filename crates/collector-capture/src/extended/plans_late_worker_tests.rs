use collector_plans::extended::plans::{
    Capacities,
    tests::{execution_capture, execution_image, pass},
};
use engine_native::walk::Outcome;

#[test]
fn completed_plans_survive_late_success_panic_stop_and_admission_rejection() {
    use crate::acquisition_jobs::{Job, LateReport, Report};
    use collector_workers::acquisition::{Failure, SubmitError};
    for mode in 0..4 {
        let image = execution_image();
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        captured.plans = execution_capture();
        pass(&image, &mut captured.plans);
        let expected = captured.plans.tables();
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut orders =
            unsafe { collector_map_capture::map_capture::MapCapacities::default().orders() };
        orders.walk_roots(&image, 0);
        orders.walk(&image, 0);
        let orders_outcome = orders.rows.outcome;
        let order_storage = orders.geometry.as_ptr();
        let countries = engine_allocation::shared(
            engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
        let bytes = captured.allocated_bytes() + orders.allocated_bytes();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            collector_workers::acquisition::Pool::start(0, 1, move |job: &mut Job| {
                let Job::Late(request) = job else {
                    panic!("only Late is submitted");
                };
                assert!(mode != 1, "synthetic Late unwind");
                request
                    .plans
                    .fail_unit_transports("synthetic optional transport refusal");
                Report::Late(LateReport)
            })
        }
        .unwrap();
        if mode == 3 {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { pool.shutdown() }.unwrap();
        }
        let request = captured.take_late_request(&countries, &mut orders, true, 0, 0, 1, false);
        assert!(engine_allocation::SharedBox::ptr_eq(
            &request.countries,
            &countries
        ));
        assert_eq!(
            captured.plans.allocated_bytes(),
            0,
            "the parked plan owner has no duplicate payload"
        );
        assert_eq!(
            captured.allocated_bytes()
                + orders.allocated_bytes()
                + request.packs.allocated_bytes()
                + request.plans.allocated_bytes(),
            bytes
        );
        let mut pending = Some(Job::Late(request));
        let submitted = unsafe { pool.submit(&mut pending) };
        let (request, completed) = if mode == 3 {
            assert_eq!(submitted, Err(SubmitError::Stopped));
            let Job::Late(request) = pending.take().unwrap() else {
                unreachable!();
            };
            (request, false)
        } else {
            let ticket = submitted.unwrap();
            if mode == 2 {
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { pool.shutdown() }.unwrap();
            }
            let completion = pool.finish(ticket).unwrap();
            match mode {
                0 => assert!(completion.report.is_ok()),
                1 => assert!(matches!(completion.report, Err(Failure::Panicked))),
                2 => assert!(matches!(completion.report, Err(Failure::Stopped))),
                _ => unreachable!(),
            }
            let completed = completion.report.is_ok();
            let Job::Late(request) = completion.request else {
                unreachable!();
            };
            (request, completed)
        };
        captured.restore_late_request(request, completed, &mut orders);
        captured.install_order_transport();
        assert_eq!(
            captured.plans.tables(),
            expected,
            "Late cannot erase a completed prefix/route/progress"
        );
        assert_eq!(orders.rows.outcome, orders_outcome);
        assert_eq!(orders.geometry.as_ptr(), order_storage);
        assert_eq!(captured.allocated_bytes() + orders.allocated_bytes(), bytes);
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
        assert!(matches!(
            captured.division_presentation.order_transport_storage().1,
            Outcome::Unavailable { .. }
        ));
        let output_bytes = captured.division_presentation.order_transport_storage().0;
        assert!(
            output_bytes > 0,
            "the fixture transferred a real sparse output buffer"
        );
        let mut capacities = Capacities::default();
        captured.plans.grow(&mut capacities);
        assert_eq!(
            captured.allocated_bytes() + orders.allocated_bytes(),
            bytes + output_bytes,
            "off-hook growth adds exactly the returned output spare, counting each owner once"
        );
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { pool.shutdown() }.unwrap();
    }
}
