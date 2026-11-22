use collector_operation_outcomes::operation_outcomes::Event;
use collector_operation_outcomes::operation_outcomes::{
    LaneCursor, handoff_buffer, invalidate,
    tests::{Fixture, complete, new_log, prime, serial},
};
use std::sync::atomic::Ordering;

#[test]
fn table_retry_preserves_admitted_operation_completion() {
    let _serial = serial();
    let log = new_log(1, 32);
    prime(&log);
    let fixture = Fixture::new(b"modded outcome");
    complete(&log, &fixture, 1);
    let mut cursors = vec![LaneCursor::default()];
    let handoff = handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)).unwrap();
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
    // SAFETY: The spool requests recovery after unpublished output in the current session.
    unsafe { shared.request_retry_full() };
    let recovery =
        // SAFETY: The test uses isolated recovery state.
        unsafe { shared.force_next() }.swap(false, Ordering::AcqRel);
    let refresh =
        // SAFETY: The test uses isolated recovery state.
        unsafe { shared.refresh_next() }.swap(false, Ordering::AcqRel);
    if recovery {
        invalidate();
    }
    let mut events = [Event::default()];
    let mut bytes = [0; 32];
    let drained = handoff.drain(&cursors, &mut events, &mut bytes).unwrap();
    assert_eq!(drained.copied, 1);
    assert_eq!(events[0].definition_token, 0xfabc_1234);
    assert_eq!(&bytes[..drained.bytes], b"modded outcome");
    assert!(refresh);
    assert!(!
        // SAFETY: This is the isolated fixture's recovery flag.
        unsafe { shared.force_next() }.load(Ordering::Acquire));
}
