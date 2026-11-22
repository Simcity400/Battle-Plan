# Collector crate layers

The collector DLL owns attach and detach, the hook type registry, hourly scheduling, frame assembly, and publication.
Domain crates own their readers and related observers, so a change to a reader can compile and test independently.
The worker crate holds the acquisition pool, stage timing, panic diagnostics, and private heap, which the DLL uses through its global allocator.
The hook failure crate holds family names, failure state and attach status, so readers can reject failed hooks without depending on the DLL registry or recording I/O.
The hook locator and capture-mode reason live with the existing hook and extended support libraries.
Mission observers sit below air and naval mission readers, and division census and order storage sit below the plan reader.
Tests that exercise frame publication or hook registration stay with the DLL and use the domains' shared fixtures.
CI builds each split crate at optimization level zero because optimizing the collector's code during checks and test builds previously dominated compile time.
