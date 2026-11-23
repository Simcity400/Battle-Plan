# Read only the recorded definitions a derivation declares

The writer framework reads numeric facts and content definitions from the recording's game facts pack. A writer declares the kinds it uses and whether each is required. Only declared kinds are read. Each runs through its own reader, and a failure or panic produces a named absence for that kind. A required absence stops its writer, while an optional absence is passed through for that writer to apply to its output. A resume gives the writer the same recorded definitions before it loads its own saved state. A repeated identical definition does not reset a writer.

The pack can be passed in memory when available or read in bounded ranges from the recording. No installed game folders are selected for those reads. Stream the province raster to count area. Use the pack's small sub-unit requirements fact rather than reading the equipment database. Read named defines in their declared form, with integer values as recorded and fixed5 values as raw integers. Resolve a province's strategic region only when exactly one distinct region claims it. A repeated claim by that same region does not introduce ambiguity.

[Definitions and readers](../../crates/recorder-derive-framework/src/framework/definitions.rs) · [Writer admission](../../crates/recorder-derive-framework/src/framework.rs)
