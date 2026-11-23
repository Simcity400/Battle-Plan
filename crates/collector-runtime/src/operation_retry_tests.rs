use collector_operation_outcomes::operation_outcomes::Event;
use collector_operation_outcomes::operation_outcomes::{
    LaneCursor, handoff_buffer, invalidate,
    tests::{Fixture, complete, new_log, prime, serial},
};
use std::sync::atomic::Ordering;

#[test]
fn table_retry_preserves_admitted_operation_completion() {
    // SAFETY: Unit test runs with no live native operation-completion hooks enabled.
    let _serial = unsafe { serial() };
    let log = new_log(1, 32);
    // SAFETY: Test holds `serial()` and exclusively owns `log` on the stack.
    unsafe { prime(&log) };
    let fixture = Fixture::new(b"modded outcome");
    // SAFETY: Test holds `serial()` and is the sole writer to `log.lanes[0]`.
    unsafe { complete(&log, &fixture, 1) };
    let mut cursors = vec![LaneCursor::default()];
    // SAFETY: `log` stays alive on the stack through the `handoff.drain` call below.
    let handoff =
        unsafe { handoff_buffer(&log, engine_native_memory::walk::Buffer::Growing(&mut cursors)) }.unwrap();
    // SAFETY: The test owns the isolated fixture state required by this constructor.
    let shared = unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
    // SAFETY: The test exercises recovery after unpublished output on its isolated `Shared`.
    unsafe { shared.request_retry_full() };
    // SAFETY: The test uses isolated recovery state.
    let recovery = unsafe { shared.force_next() }.swap(false, Ordering::AcqRel);
    // SAFETY: The test uses isolated recovery state.
    let refresh = unsafe { shared.refresh_next() }.swap(false, Ordering::AcqRel);
    if recovery {
        // SAFETY: Test holds `serial()` and exercises recovery invalidation in isolation.
        unsafe { invalidate() };
    }
    let mut events = [Event::default()];
    let mut bytes = [0; 32];
    // SAFETY: `log` is still alive and `cursors` came from `handoff_buffer` above.
    let drained = unsafe { handoff.drain(&cursors, &mut events, &mut bytes) }.unwrap();
    assert_eq!(drained.copied, 1);
    assert_eq!(events[0].definition_token, 0xfabc_1234);
    assert_eq!(&bytes[..drained.bytes], b"modded outcome");
    assert!(refresh);
    assert!(!
        // SAFETY: This is the isolated fixture's recovery flag.
        unsafe { shared.force_next() }.load(Ordering::Acquire));
}
