# Record the first escaping decode panic

The decode worker and the detach drain use the same scoped panic context. Per-frame context comes from the owned decode lease. The process panic hook formats a diagnostic on failure, while the phase and source location are still available. A panic caught within a decode stage must not replace the cause of a terminal failure. Only an escaping worker panic or its join fallback confirms the diagnostic, and the first confirmation wins.

The loader owns the hook for the workers' lifetime. Quiescent teardown joins the workers and restores the original hook closure so the process retains no pointer into an unloaded collector. If the collector must stay pinned, it keeps the hook owner with the workers. The join and drain fallbacks retain a panic payload without invoking its destructor, since that destructor can panic during teardown.

The payload retirement queue likewise lives through quiescent loader teardown. A frame that never returns its borrowed orders buffer retires it through the original job family exactly once, on either an ordinary drop or an unwind. The orders regression builds its naval job from the public default because the capture state is private.

[Decode failure context](../../crates/collector-workers/src/panic_failure.rs) · [Order loan regression](../../crates/collector-capture/src/decode/order_group_loan_tests.rs)
