# 0402. Read current and completed focus from one authenticated country root

The current focus and completed roster use the same authenticated progress object. The reader retains the complete signed fixed-point progress even while the country has no active focus. It carries the native pause byte and preserves the completed-focus vector's order. The game's daily update publishes these values before the hourly callback returns, so acquisition reads the existing state and requires no completion hook.

Summary and completed-definition reads can fail independently after root authentication. Where available, advisory worker storage lets the reader batch exact fields. A failed speculative batch permits one pass through the serial reader. The caller authenticates and enumerates countries and owns each output buffer. A token is stored in full or omitted, and the first omitted token contributes to the byte demand so the worker can grow the arena without reading the missing suffix.

Synthetic governance fixtures preserve a separate layout for country selection fields and their pointed-to data. Guarded-call counters use thread-local baselines. One field-only gather counts as one guarded entry for all its operands. One-shot faults target the speculative gathers without affecting the serial oracle. The focus oracle keeps the frozen serial implementation and checks a failed summary definition and a failed completed definition separately.

[Focus acquisition](../../crates/engine-focus/src/cnational_focus_progress.rs) · [Batch tests](../../crates/engine-focus/src/cnational_focus_progress/batch_tests.rs) · [Governance fixtures](../../crates/engine-governance-tests/src/kit.rs)
