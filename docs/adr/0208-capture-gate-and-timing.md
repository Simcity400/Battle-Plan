# Hook diagnostics separate collector timing from the native original

## Capture gate

The collector needs a settled simulation state while it reads. Game writers arrive through the command batch and the hooked writers it reaches. A writer arriving during a capture waits unless it is a collector reader or every reader of that writer's fields uses an observation from capture start. A family can revoke that exemption at a particular entrance and wait through the same path, including the same diagnostics. Waiting gives the capture a hurry signal, and a waiting thread retains the collector until it exits. The waiter increments the count before testing state. The capture clears state before testing the count, so sequentially consistent ordering makes either the waiter see the completed capture or the capture see the waiter. A bounded wait also rechecks for a missed wake.

Wait counters are cumulative from attach. Slots distinguish their families, with later callers sharing the final slot. Tests isolate the writer family, exemption and thread marker from other tests using the process-wide gate.

`end` passes the address of the process-lifetime, aligned `u32` in `STATE` to `WakeByAddressAll`. `wait_out` passes the same address to `WaitOnAddress`, with a local `u32` comparison value that stays live throughout the call and a comparison size of `size_of::<u32>()`. The timeout bounds a missed wake.

## Timing and status

Each installed hook family owns a timing identity and a gate slot. Keep hourly boundaries separate from the between-tick detours that happen within the native hourly call. Ordinary detours time their originals separately. Interior taps have no original call inside the span. When assembly or C++ owns the native call, time the entry and return halves and omit the interval between them. A mixed family can have both original calls and pre-observers. The return half does not add a callback count.

Thread ownership keeps the slot single-writer and cache-aligned. Calls beyond the owned slots use atomic increments in a shared slot. The first and every selected later call are sampled per thread and family. Time every hourly boundary because it runs once per game hour. Sampled timing reads occur after the instructions they close. Keep the span on its owning thread and keep its `Hook` allocation phase until it drops. Code outside the timing span can take that phase by itself. Separate diagnostic call counters share thread ownership, and concurrent status reads can include different observation times. Their diagnostic counts cannot certify acquisition.

At startup, the production timing path estimates overhead using the fastest empty span and the fastest span with an empty timed original. Both values are published together. Each sampled call caps the estimate at its observed detour. Status scales each slot by its own calls per sample, then sums slots. The timer adjustment is approximate and may subtract some collector work, so keep raw figures and label the adjusted values `approximate_*`. Calibration uses TSC and QPC with a minimum interval to bound read skew. The first origin wins and is established before the hooks can run.

`Family` aggregates related entry points into metrics, including advice and character roles, diplomacy, script flags and variables, construction and production, military transport and members, doctrine and world operands, and naval and land events. A family with its own status reuses that key. Preserve separate identities for political-power taps, event-feed provenance, and `CommandBatch` so their counts remain attributable.

Borrowed status construction allocates no heap storage. JSON formatting streams without building an intermediate string. A reused buffer keeps a sticky allocation failure out of serde's boxed I/O-error path. In `LossyBytes::fmt`, `from_utf8_unchecked` receives only the prefix ending at `Utf8Error::valid_up_to()`, which the preceding `from_utf8` checked.

The consuming packages exercise the shared collector test helpers. In their harness, allocation denial wraps the prepared-buffer child interval and keeps its own independent count per test thread. Process setup, thread creation, paths and recovery stay outside this interval. Per-call timing takes the median after a warmup. `reserve_test_stack` sets `RUST_MIN_STACK` only from a test executable CRT initializer before `main`, while the calling thread is the only thread that can access the process environment.

[Capture gate](../../crates/collector-timing/src/capture_gate.rs) · [Hook timing](../../crates/collector-timing/src/hook_timing.rs) · [Status formatting](../../crates/collector-status/src/status.rs) · [Test allocator](../../crates/collector-test-support/src/lib.rs)
