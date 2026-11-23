# Prepared capture resumes from its first observed prefix

The collector can retain an owned result when replaying its exact ordered control observations certifies it. Compilation happens on recycle alongside row and arena growth. A control change or a failed prepared read leaves a prefix that the ordinary reader consumes in order, including the failed observation. An ordinary walk must not retry those reads. A record gather can run as a gather outside that prefix. Inside it, each field is consumed in row-major order to preserve the ordinary reader's observation boundary.

Callers can track data reads without using them as control comparisons. Those reads remain fresh and fault-checked in the original order. They cannot determine subsequent addresses or their validation. The completed buffer exposes them only after a fully completed execution, and the caller applies them before publishing its retained result. Any omitted observations need independent coverage, such as uninterrupted mutation revisions.

Raw capture and prepared traces validate read extents before touching memory. An empty extent succeeds. Nonempty spans require both endpoints to be in the same canonical half, with bit 47 included in the sign extension and overflow treated as invalid. Early validation attributes the failed read to its requested range. The guarded copy still contains native faults for direct callers.

[Snapshot crate](../../crates/collector-snapshots/src/lib.rs) · [Prepared read plans](../../crates/collector-snapshots/src/snapshot_plan.rs) · [Range gate](../../crates/engine-native-memory/src/memory.rs)
