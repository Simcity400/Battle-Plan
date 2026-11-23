# Track derived logs by append ordinal and declared key lifetime

For the first read of a run, a log reader receives the listed rows as backlog. Later it receives new appends and forgotten ordinal runs. An in-place replacement forgets the earlier ordinal and appends the same logical key. A consumer can update its keyed state or count the new append, depending on the product. No source row cells survive a forget, so a derivation that needs them stores the relevant fields in its own table at append time. The minimal restored log state is the count and the listed ordinals. `prime` receives no appends.

A writer stages appends and forgets for the hour and lands them through the log path on success. Appends take ordinals in insertion order, then forgets are applied as ordered ordinal runs. A writer can forget rows appended in the same hour. It cannot forget an unavailable or already forgotten ordinal, write logs as state, or reuse a key within its declared lifetime.

Run-keyed logs retain keys in ranges and persist them across block resumes. Hour-keyed logs carry the write hour in their key, verify it on append, and retain keys only for that hour. This avoids growing a historical key set for a product whose key space already separates hours. The tests replay appends and forgets across blocks, then compare seeks and resumed output with the uninterrupted recording for both key lifetimes.

[Log contracts and implementation](../../crates/recorder-derive-framework/src/framework.rs) · [Log parity and failure fixtures](../../crates/recorder-derive-framework/src/framework/tests.rs)
