use collector_states::{
    acquisition_states::tests::Movement,
    extended::state_flags::{
        Capacities, Captured,
        tests::{NoReads, STATE, memory, request_layout},
    },
};
use engine_native::{Memory, cgame_state, cstate, memory::SliceMemory, walk::Outcome};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn state_flags_pool_failure_restores_all_three_buffers_without_native_retry() {
    use collector_states::acquisition_states::{Report, Request};
    use collector_workers::acquisition::{Failure, Pool};
    struct PanicAtRoot<'a> {
        bytes: &'a [u8],
        roots: Cell<usize>,
        calls: &'a AtomicUsize,
    }
    impl Memory for PanicAtRoot<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if address == STATE + cgame_state::STATES {
                let count = self.roots.get() + 1;
                self.roots.set(count);
                assert_ne!(count, 2, "synthetic state census panic");
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { collector_capture::sampling::Captured::new() };
    for canceled in [false, true] {
        frame.states = vec![cstate::Row::default(); 4];
        frame
            .extended
            .restore_state_visitor(Captured::with_capacities(&Capacities { rows: 1 }));
        let request = frame.take_states_request(0, STATE);
        let buffers = request_layout(&request);
        let vacant = frame.take_states_request(0, STATE);
        assert!(
            request_layout(&vacant)
                .iter()
                .all(|&(_, len, capacity)| len == 0 && capacity == 0)
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let runner_calls = Arc::clone(&calls);
        let bytes = memory();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(0, 1, move |request: &mut Request| {
                request.run(&PanicAtRoot {
                    bytes: &bytes,
                    roots: Cell::new(0),
                    calls: &runner_calls,
                })
            })
        }
        .unwrap();
        let mut pending = Some(request);
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        if canceled {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { pool.shutdown() }.unwrap();
        }
        let completed = pool.finish(ticket).unwrap();
        assert!(
            matches!(completed.report, Err(failure) if failure == if canceled { Failure::Stopped } else { Failure::Panicked })
        );
        let demand = completed.request.visitor.required;
        assert_eq!(demand, usize::from(!canceled));
        let calls_before = calls.load(Ordering::Relaxed);
        let reason = "state census worker failed";
        let report =
            frame.restore_states_request(completed.request, Report::default(), Some(reason));
        assert_eq!(frame.states_result, Outcome::Unavailable { reason });
        assert_eq!(report.ticks, 0);
        let mut restored = frame.take_states_request(0, STATE);
        assert_eq!(request_layout(&restored), buffers);
        assert_eq!(restored.visitor.source, Outcome::Unavailable { reason });
        assert_eq!(restored.visitor.required, demand);
        assert!(
            restored
                .visitor
                .first
                .iter()
                .all(|source| *source == cstate::Source::default())
        );
        restored.visitor.capture(&NoReads, 0, STATE);
        assert_eq!(
            restored.visitor.outcomes(),
            [Outcome::Unavailable { reason }]
        );
        assert_eq!(request_layout(&restored), buffers);
        assert_eq!(restored.visitor.required, demand);
        assert_eq!(calls.load(Ordering::Relaxed), calls_before);
        assert_eq!(pool.calibration().uncollected, 0);
        frame.restore_states_request(restored, report, None);
    }
}

#[test]
fn state_flags_successful_states_restore_preserves_timing_and_storage() {
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { collector_capture::sampling::Captured::new() };
    frame.states = vec![cstate::Row::default(); 1];
    frame.state_inputs = collector_states::acquisition_states::Inputs::with_capacity(1).unwrap();
    frame
        .extended
        .restore_state_visitor(Captured::with_capacities(&Capacities { rows: 1 }));
    let bytes = memory();
    let memory = Movement(SliceMemory {
        base: 0,
        bytes: &bytes,
    });
    frame.extended.reset();
    let mut request = frame.take_states_request(0, STATE);
    let buffers = request_layout(&request);
    let mut report = request.run(&memory);
    assert_eq!(report.census.result, Outcome::Rows { len: 1 });
    report.ticks = 137;
    let restored = frame.restore_states_request(request, report, None);
    assert_eq!(restored.ticks, 137);
    assert_eq!(frame.states_result, Outcome::Rows { len: 1 });
    let mut request = frame.take_states_request(0, STATE);
    assert_eq!(request_layout(&request), buffers);
    assert_eq!(
        request.visitor.source,
        collector_extended_support::outcomes::MISSING
    );
    assert_eq!(request.visitor.outcomes(), [Outcome::Rows { len: 1 }]);
    request.visitor.capture(&NoReads, 0, STATE);
    assert_eq!(request.visitor.outcomes(), [Outcome::Rows { len: 1 }]);
    assert_eq!(request_layout(&request), buffers);
    assert!(
        request
            .visitor
            .first
            .iter()
            .all(|source| source.pointer == 0)
    );
    frame.restore_states_request(request, restored, None);
}
