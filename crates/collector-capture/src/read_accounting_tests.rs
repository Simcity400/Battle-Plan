use collector_memory::read_accounting::{
    fixtures::{ReadTrace, copies, read},
    *,
};
use replay_data::rows::TableId;

#[test]
fn executor_completion_preserves_exact_counts_on_workers_callers_and_panics() {
    use std::sync::{Arc, mpsc};

    for workers in [0, 2] {
        let trace = Arc::new(ReadTrace::default());
        let runner_trace = Arc::clone(&trace);
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            collector_workers::acquisition::Pool::start(
                workers,
                4,
                move |job: &mut (usize, bool)| {
                    entered_tx.send(std::thread::current().id()).unwrap();
                    runner_trace.record(job.1, || {
                        with_table(TableId::States, || read(job.0));
                        with_table(TableId::StateFlags, || read(job.0 + 1));
                        if job.1 {
                            panic!("synthetic owned-job failure");
                        }
                        job.0
                    })
                },
            )
        }
        .unwrap();
        for fail in [false, true] {
            let frame = Capture::new();
            read(2);
            let mut request = Some((3, fail));
            let ticket = unsafe { pool.submit(&mut request) }.unwrap();
            let worker_thread = (workers != 0).then(|| {
                entered_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap()
            });
            let completed = pool.finish(ticket).unwrap();
            let executing_thread = worker_thread.unwrap_or_else(|| entered_rx.try_recv().unwrap());
            assert_eq!(
                executing_thread == std::thread::current().id(),
                workers == 0
            );
            assert_eq!(completed.report.is_err(), fail);
            let mut expected = ZERO;
            expected[TableId::States.index()] = copies(1, 3);
            expected[TableId::StateFlags.index()] = copies(1, 4);
            let job_reads = trace.take(completed.request.1);
            assert_eq!(job_reads, expected);
            #[cfg(feature = "acquisition_profile")]
            assert_eq!(completed.table_reads, job_reads);
            let mut entry = ZERO;
            entry[TableId::Clock.index()] = copies(1, 2);
            assert_eq!(snapshot(), entry);
            merge(&job_reads);
            expected[TableId::Clock.index()] = entry[TableId::Clock.index()];
            assert_eq!(snapshot(), expected);
            assert!(pool.finish(ticket).is_none());
            assert_eq!(snapshot(), expected);
            assert_eq!(frame.finish(), expected);
        }
        let calibration = pool.calibration();
        assert_eq!(calibration.accepted, 2);
        assert_eq!(calibration.completed, 2);
        assert_eq!(calibration.panicked, 1);
        assert_eq!(calibration.stopped, 0);
        assert_eq!(calibration.uncollected, 0);
        assert_eq!(
            calibration.caller_executed,
            if workers == 0 { 2 } else { 0 }
        );
        assert_eq!(
            calibration.worker_executed,
            if workers == 0 { 0 } else { 2 }
        );
        pool.settle();
        assert_eq!(pool.calibration().active, 0);
    }
}
