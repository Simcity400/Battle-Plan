# collector-projection

Checked projection and conversion of owned native rows into replay table outcomes. Sorting, exact row counts, duplicate refusals, cached comparisons and terminal allocation guards are shared by capture packs and spool composition.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-projection` and `cargo clippy -p collector-projection --all-targets -- -D warnings` for its synthetic checks.

Certified immutable row stores retain path-copied ordered roots and cached lengths and byte counts. A capture publishes only changed key ranges, with no complete-vector traversal on its acquisition worker. Projection binds the revision to the encoder's committed branch and hour; gaps, failed commits and keyframes materialise a complete vector on the spool thread. Quiet snapshots use the same ownership.

A withdrawal can fork an immutable snapshot and replace selected key ranges while sharing independent paths. Range visitors inspect only those dependencies. Each fork has its own revision, so publication cannot confuse a refusal baseline with the original successful values.

Snapshot range-presence queries follow only an ordered tree path and allocate nothing, so publication guards can check actual dependent rows without scanning or trusting acquisition bookkeeping.
Complete writer certificates can select owner replacements or exact holder-key
replacements against the encoder's shared baseline. Exact contract/agreement
updates avoid publishing unaffected holders belonging to the same owner.
Periodic and forced encoding keyframes materialize complete state from that
baseline and selected changes, without native acquisition. Carried collections
stay shared; empty changes reuse the baseline Arc. Missing/partial baselines,
branch changes and hour gaps still refuse carry. Ordinary hours remain sparse.
