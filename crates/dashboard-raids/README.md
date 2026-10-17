# dashboard-raids

The raids product: one observer of every country's raid launches and results,
shared by the raid log and the raid tally. Behaviour is documented beside the
code in [products/raids.rs](src/products/raids.rs) and in the widget guides
([raid log](../../ui/src/widgets/raid-log/README.md), [raid
tally](../../ui/src/widgets/raid-tally/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md),
laid out like
[dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the module
keeps its dashboard path and names what it uses at the crates that own it
(dashboard-base, dashboard-text and dashboard-core). The
[dashboard](../dashboard/README.md) re-exports it at its old
`dashboard::products::` path, registers it, and prepares and queries it in its
pipeline.

`cargo test -p dashboard-raids` builds and runs only this crate's tests. The
`test-support` feature compiles the whole-recording loader, the parity oracle
the dashboard's pipeline tests compare against.

The tally answers from the raid and country-identity joint prepared prefix; no
counts are assigned to its pending hours. The log combines its prefix with a
proven newest block. A two-block observation is exact when none of its native
raid keys occurs earlier: launch witnesses and frozen ended victims persist by
that key, so an earlier occurrence leaves the newest block pending. Earlier keys
are change-read in parallel and retained per block in the shared source-revision
observation cache. Compressed newest proofs use the same cache in a content-bound
scope. Both use its live-block retention bound; recording/content changes
invalidate them, while coverage re-asks reuse retained observations.
