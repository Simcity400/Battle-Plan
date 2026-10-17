# dashboard-special-projects

The special projects product: each country's project facilities, progress,
prototypes and completions. Its behaviour is documented beside the code in
[products/special_projects.rs](src/products/special_projects.rs) and in the
[special projects widget guide](../../ui/src/widgets/special-projects/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::special_projects`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)). The [dashboard](../dashboard/README.md)
re-exports the module at `dashboard::products::special_projects`, registers it,
and prepares and queries it in its pipeline.

The block pre-pass reads country presentation and trigger tables only when
the bound content has a scoped project name. Unscoped names read the project,
program, history, character, portrait, dictionary and country identity tables.
The extract finds its next completion deadline again only when history changes
or the previous deadline is reached; every due completion is still observed.

`cargo test -p dashboard-special-projects` builds and runs only this product's
tests. The `test-support` feature compiles its test oracles and instrumentation
for the dashboard's tests.

Current requests name an hour. Running cards before history reaches that hour use the frame, the preparation observer and its episode/presentation builders. Starts and completed/cancelled history remain preparing. The separate `current` reply cannot enter History lanes or expose a fabricated start date.

Current-card selections include exact holders witnessed in the requested frame, including countries not yet reached by the identity prefix.

Country identity comparisons follow recorded country field masks: balance-only
deltas and equal acquisitions keep the previous key projection. Snapshots,
identity edits and membership changes use the complete key comparison.

The supplying-read marker carries equal authenticated values between changes.
Unknowns and reactivation take ordinary observation. Structural project reads
compare held numeric values; publication materializes their endpoints after
the hour's changes are observed.
