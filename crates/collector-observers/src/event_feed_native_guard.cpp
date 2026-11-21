#include <windows.h>
#include "event_feed_native_guard.h"

extern "C" __declspec(noinline)
void hoi4_event_native_guard(EventFrame* frame) {
    uint32_t observed = 0;
    __try {
        __try {
            observed = hoi4_event_before(frame);
        }
        __except (EXCEPTION_EXECUTE_HANDLER) {
            hoi4_event_observer_fault();
        }

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
            hoi4_event_native_escape();
        }
    }
}
