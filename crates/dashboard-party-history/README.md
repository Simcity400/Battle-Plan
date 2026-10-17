# dashboard-party-history

The party history product: one country's politics over a branch, each party's popularity, which party ruled and when elections were held. Behaviour is documented beside the code in [products/party_history.rs](src/products/party_history.rs) and in the widget guide [ruling party](../../ui/src/widgets/ruling-party/README.md).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
module keeps its dashboard path, `products::party_history`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports it at the old
`dashboard::` paths, registers it, and prepares and queries it in its pipeline.

`cargo test -p dashboard-party-history` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.

Progressive popularity summaries retain each block's sampled fold with its
earliest duplicate-popularity evidence. Re-asks restore that evidence with the
fold, preserving the existing overlap refusal. State and navigation samplers
keep their ordinary walks and diagnostics.

The party observer retains country keys when the reduced allocation holds or
recorded deltas change only balance fields. Snapshots, membership changes and
identity edits compare the complete key projection before joining holders.
