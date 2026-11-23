# 1115. Preserve transient recording events across playback steps

A nuclear strike can be appended and removed before the next publication. The strikes layer consumes the appended rows captured by each step, in birth order, to start every effect the head passed. It independently draws the impacts still listed at the applied hour. Both use the province's standstill placement, while an impact without a known placement is counted. A reset starts the sought hour's impacts with the previous effect slots cleared.

For budgeted layer folds, insertions and deletions are marked by row key and affect every column group. Cell writes are marked by slot and touched groups. The settle resolves those slots against the current mirror: a reused slot now identifies the replacement row. The insertion and deletion marks cover the old and new identities independently. This preserves step order without making the apply thread read mirrors for work it may carry.

[Strikes](../../crates/viewer-layers-infrastructure/src/strikes.rs) · [Touched rows](../../crates/viewer-layer-core/src/touched.rs)
