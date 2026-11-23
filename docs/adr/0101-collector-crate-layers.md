# Collector crate layers

The collector DLL owns the Windows entry point, the exported callback symbols and the global allocator. The allocator forwards to the worker crate's private heap. [DLL entry point](../../crates/collector/src/lib.rs)

Runtime owns attachment, detachment, the hook type and site registries, and the worker and status lifecycles. [Runtime](../../crates/collector-runtime/src/runtime.rs)

Cross-crate lifecycle operations use `unsafe fn` contracts that reserve startup and teardown for the runtime's attach and detach sequence. The contracts preserve the split's existing behaviour.

Capture owns hourly admission and scheduling, the pack registry, frame assembly, and decode. Map capture and player command observation each have a separate crate. The player command observer uses replay's command record types through a dependency with default features disabled. Capture supplies the observer with its roster sampling callback. [Hourly capture](../../crates/collector-capture/src/sampling.rs)

Spooling consumes captured frames and owns conversion, writing, and history replies. The pack registry owns the part of conversion that enumerates captured pack types. [Spool processing](../../crates/collector-spool/src/spooling.rs)

The worker crate holds the acquisition pool, worker policy and diagnostics, stage timing, frame fencing, and spool helpers. Its helper limit is a literal checked at compile time against the capture registry's helper count. [Workers](../../crates/collector-workers/src/lib.rs) ? [Helper count check](../../crates/collector-capture/src/pack_conversion.rs)

Domain crates own their readers and related observers, so a change to a reader can compile and test independently.
The hook failure crate holds family names, failure state and attach status, so readers can reject failed hooks without depending on the runtime registry or recording I/O. Runtime supplies the site inventory when reporting a failure.
The hook locator and capture-mode reason live with the hook and extended support libraries.
Mission observers sit below air and naval mission readers, and division census and order storage sit below the plan reader.

Tests live with the crate that owns the predicate they check. The DLL omits the test harness.
CI builds each split crate at optimization level zero because optimizing the collector's code during checks and test builds previously dominated compile time.
