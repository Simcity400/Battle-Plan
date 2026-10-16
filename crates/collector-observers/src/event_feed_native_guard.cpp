// Catch injected observer failures while preserving native exceptions.
#include <windows.h>
#include "event_feed_native_guard.h"

extern "C" __declspec(noinline)
void hoi4_event_native_guard(EventFrame* frame) {
    uint32_t observed = 0;
    __try {
        // Observer failure retains ownership; the original must still run.
        __try {
            observed = hoi4_event_before(frame);
        }
        __except (EXCEPTION_EXECUTE_HANDLER) {
            hoi4_event_observer_fault();
        }

        // Native exceptions skip both observer filters and propagate unchanged.
        hoi4_event_forward(frame);

        if (observed == 1) {
            __try {
                hoi4_event_returned(frame);
            }
            __except (EXCEPTION_EXECUTE_HANDLER) {
                hoi4_event_observer_fault();
            }
        }
    }
    __finally {
        if (AbnormalTermination()) {
            // Retain flights, tickets and storage after any native escape.
            hoi4_event_native_escape();
        }
    }
}
