# Preserve native fleet sort order

Fleet displays sort by a wrapping signed difference between handle words. That relation can be nontransitive, which means two stable sorting implementations can produce different orders for the same input. Keep the native algorithm's insertion cutoff, paired buffered passes and merge comparisons so a cyclic triple follows the native sequence. The tests cover both the small insertion path and the merged path, where every item must still appear exactly once.

[Fleet sorting](../../crates/viewer-runtime/src/msvc_stable_sort.rs) · [Naval combat grid](../../crates/viewer-counter-kit/src/naval_combat_grid.rs) · [Regional naval counters](../../crates/viewer-counters-naval/src/naval_regional.rs)
