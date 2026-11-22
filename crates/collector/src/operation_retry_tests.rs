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
    // SAFETY: Test holds `serial()` and exclusively owns `log` on the stack.
    unsafe { prime(&log) };
    let fixture = Fixture::new(b"modded outcome");
    complete(&log, &fixture, 1);
    let mut cursors = vec![LaneCursor::default()];
    // SAFETY: `log` stays alive on the stack through the `handoff.drain` call below.
    let handoff =
        unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }.unwrap();
    let shared = crate::sampling::Shared::new(0, 0, 1);
    shared.request_retry_full();
    let recovery = shared.force_next.swap(false, Ordering::AcqRel);
    let refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
    if recovery {
        invalidate();
    }
    let mut events = [Event::default()];
    let mut bytes = [0; 32];
    // SAFETY: `log` is still alive and `cursors` came from `handoff_buffer` above.
    let drained = unsafe { handoff.drain(&cursors, &mut events, &mut bytes) }.unwrap();
    assert_eq!(drained.copied, 1);
    assert_eq!(events[0].definition_token, 0xfabc_1234);
    assert_eq!(&bytes[..drained.bytes], b"modded outcome");
    assert!(refresh);
    assert!(!shared.force_next.load(Ordering::Acquire));
}
