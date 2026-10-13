use super::*;
use crate::{acquisition_jobs::OrdersRequest, map_capture::Orders, raw::RawLog};
use engine::memory::SliceMemory;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
#[ignore = "isolated shared retirement lifecycle; run with --test-threads=1"]
// NavalJob's private capture state requires constructing this fixture via Default.
#[allow(clippy::field_reassign_with_default)]
fn abandoned_orders_loan_retires_once_on_drop_and_callback_unwind() {
    let _serial = crate::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prepare_retirement();
    reclaim_payloads();
    let queue = RETIRED_PAYLOADS.load(Ordering::Acquire);
    // Preparation keeps the queue alive until quiescent loader teardown.
    let queue = unsafe { &*queue };
    assert!(queue.is_empty());
    for unwind in [false, true] {
        let mut payload = NavalJob::default();
        payload.log = RawLog::new(4, 32);
        let marker = [17_u8, 23];
        assert!(
            payload
                .log
                .take(
                    &SliceMemory {
                        base: 0x30000,
                        bytes: &marker
                    },
                    0x30000,
                    marker.len(),
                    0x1C0
                )
                .is_some()
        );
        let identity = payload.log.bytes.as_ptr();
        let request = OrdersRequest {
            countries: std::sync::Arc::new(
                engine::country_memory::Cache::try_with_capacity(0).unwrap(),
            ),
            base: 0,
            game_state: 0,
            orders: Orders::vacant(),
            plans: crate::extended::plans::Captured::vacant(),
            group_source: Some(payload),
        };
        let result = catch_unwind(AssertUnwindSafe(move || {
            let owner = request;
            assert!(!unwind, "synthetic callback-wide unwind before loan return");
            drop(owner);
        }));
        assert_eq!(result.is_err(), unwind);
        let Some(RetiredPayload::Naval(payload)) = queue.pop() else {
            panic!("abandoned Naval loan must retire through its original family");
        };
        assert_eq!(payload.log.bytes.as_ptr(), identity);
        assert_eq!(payload.log.bytes.as_ref(), &marker);
        assert!(queue.is_empty());
        // This abandoned frame is retired, rather than decoded as accepted.
        drop(payload);
    }
}
