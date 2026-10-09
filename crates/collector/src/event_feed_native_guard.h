// Native guard and Rust stack frame share this layout.
#pragma once
#include <stddef.h>
#include <stdint.h>

struct EventFrame {
    uintptr_t rsp_before_call;
    uintptr_t rcx, rdx, r8, r9, rdi, rbp, rax, r10, r11;
    uint8_t xmm[6][16];
    uint32_t site, owned_flight, terminal, reserved;
    // Opaque Rust MaybeUninit<Ticket>. C++ must never inspect/settle it.
    uint64_t ticket[2];
    uint64_t generation;
    uintptr_t target;
};
static_assert(sizeof(EventFrame) == 224);
static_assert(offsetof(EventFrame, xmm) == 80);
static_assert(offsetof(EventFrame, ticket) == 192);
static_assert(offsetof(EventFrame, target) == 216);

extern "C" uint32_t hoi4_event_before(EventFrame*);
extern "C" void hoi4_event_returned(EventFrame*);
extern "C" void hoi4_event_forward(EventFrame*);
extern "C" void hoi4_event_observer_fault(void);
extern "C" void hoi4_event_native_escape(void);
